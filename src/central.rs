#![no_main]
#![no_std]

use panic_probe as _;
use rmk::macros::rmk_central;

mod display_spi;
mod nice_view;
mod screen;

embassy_nrf::bind_interrupts!(struct DisplayIrqs {
    SPI2 => embassy_nrf::spim::InterruptHandler<embassy_nrf::peripherals::SPI2>;
});

#[rmk_central]
mod keybaord_central {
    #[register_processor(poll)]
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
        ScreenProcessor::<_, _, _, _, Central>::new(panel)
    }

    // RMK 0.9 has no post-keymap processor-init hook. Retain macro-generated
    // hardware, storage, keymap, keyboard, host, ADC and watchdog initialization;
    // only its final task entry is overridden to seed the restored active layer.
    // Keep this task list in sync with keyboard.toml and RMK's entry generator.
    #[Override(entry)]
    fn run_with_initial_screen_state() {
        status_screen.seed_from_keymap(&keymap);
        let mut wpm_processor = rmk::processor::builtin::wpm::WpmProcessor::new();
        let mut usb_transport = rmk::usb::UsbTransport::new(driver, rmk_config.device_config)
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
            wpm_processor,
            watchdog_runner,
            battery_processor,
        )
        .await;
    }
}
