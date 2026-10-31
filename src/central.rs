#![no_main]
#![no_std]

use panic_probe as _;
use rmk::macros::rmk_central;

mod clock_config;
mod display_spi;
mod nice_view;
mod screen;
mod usb_power;

embassy_nrf::bind_interrupts!(struct DisplayIrqs {
    SPI2 => embassy_nrf::spim::InterruptHandler<embassy_nrf::peripherals::SPI2>;
});

#[rmk_central]
mod keybaord_central {
    // Keep resource allocation and startup order aligned with RMK's nRF chip initialization.
    #[Override(chip_init)]
    fn chip_init() {
        use embassy_nrf::interrupt::InterruptExt;
        let p = embassy_nrf::init(crate::clock_config::embassy_config());
        let mpsl_p = nrf_sdc::mpsl::Peripherals::new(
            p.RTC0, p.TIMER0, p.TEMP, p.PPI_CH19, p.PPI_CH30, p.PPI_CH31,
        );
        static MPSL: static_cell::StaticCell<nrf_sdc::mpsl::MultiprotocolServiceLayer> =
            static_cell::StaticCell::new();
        static SESSION_MEM: static_cell::StaticCell<nrf_sdc::mpsl::SessionMem<1>> =
            static_cell::StaticCell::new();
        let mpsl = MPSL.init(defmt::unwrap!(
            nrf_sdc::mpsl::MultiprotocolServiceLayer::with_timeslots(
                mpsl_p,
                Irqs,
                crate::clock_config::mpsl_config(),
                SESSION_MEM.init(nrf_sdc::mpsl::SessionMem::new()),
            )
        ));
        spawner.spawn(mpsl_task(&*mpsl).unwrap());
        let sdc_p = nrf_sdc::Peripherals::new(
            p.PPI_CH17, p.PPI_CH18, p.PPI_CH20, p.PPI_CH21, p.PPI_CH22, p.PPI_CH23, p.PPI_CH24,
            p.PPI_CH25, p.PPI_CH26, p.PPI_CH27, p.PPI_CH28, p.PPI_CH29,
        );
        let mut rng = embassy_nrf::rng::Rng::new(p.RNG, Irqs);
        let mut sdc_mem = nrf_sdc::Mem::<
            {
                rmk::ble::calculate_sdc_memory_size(
                    1,
                    rmk::ble::NUM_HOST_LINKS,
                    SDC_PACKET_SIZE,
                    SDC_TX_BUFFERS,
                    SDC_RX_BUFFERS,
                )
            },
        >::new();
        let ble_controller = defmt::unwrap!(build_sdc(sdc_p, &mut rng, &*mpsl, &mut sdc_mem));
        let ble_addr = crate::clock_config::ble_address();
    }

    #[register_processor]
    fn status_screen() -> impl rmk::processor::PollingProcessor {
        use crate::screen::runtime::{Central, ScreenProcessor};
        use embassy_nrf::{
            gpio::{Level, Output, OutputDrive},
            spim,
        };
        let mut config = spim::Config::default();
        config.frequency = spim::Frequency::M1;
        config.mode = spim::MODE_0;
        config.bit_order = spim::BitOrder::LsbFirst;
        let spi = crate::display_spi::DisplaySpi::new(
            p.SPI2,
            crate::DisplayIrqs,
            p.P0_20,
            p.P0_17,
            config,
        );
        let cs = Output::new(p.P0_06, Level::Low, OutputDrive::Standard);
        let power = Output::new(p.P0_13, Level::High, OutputDrive::Standard);
        let panel = crate::nice_view::NiceView::new(spi, cs, power, embassy_time::Delay);
        ScreenProcessor::<_, _, _, _, Central>::new(panel, crate::usb_power::is_usb_power_present)
    }

    // RMK 0.9 has no post-keymap processor-init hook. Retain macro-generated
    // hardware, storage, keymap, keyboard, host, ADC and watchdog initialization;
    // only its final task entry is overridden to seed the restored active layer.
    // Keep this task list in sync with keyboard.toml and RMK's entry generator.
    #[Override(entry)]
    fn run_with_initial_screen_state() {
        status_screen.seed_from_keymap(&keymap);
        let mut usb_transport =
            rmk::usb::UsbTransport::new(driver, rmk_config.device_config.clone())
                .with_host_service(&host_service);
        let mut ble_transport = rmk::ble::BleTransport::new(
            ble_controller,
            ble_addr,
            rmk_config,
            [rmk::split::PeripheralMatrixConfig {
                rows: 4,
                cols: 5,
                row_offset: 0,
                col_offset: 5,
            }],
        )
        .with_host_service(&host_service);
        rmk::run_all!(
            adc_device,
            matrix,
            storage,
            keyboard,
            status_screen,
            usb_transport,
            ble_transport,
            watchdog_runner,
            battery_processor,
        )
        .await;
    }
}
