#![no_main]
#![no_std]

use panic_probe as _;
use rmk::macros::rmk_peripheral;

mod display_spi;
mod nice_view;
mod screen;

embassy_nrf::bind_interrupts!(struct DisplayIrqs {
    SPI2 => embassy_nrf::spim::InterruptHandler<embassy_nrf::peripherals::SPI2>;
});

#[rmk_peripheral(id = 0)]
mod keyboard_peripheral {
    #[register_processor(poll)]
    fn status_screen() -> impl rmk::processor::PollingProcessor {
        use crate::screen::runtime::{Peripheral, ScreenProcessor};
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
        ScreenProcessor::<_, _, _, _, Peripheral>::new(panel)
    }
}
