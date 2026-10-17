//! Queued HAL expectations for the real panel transport. Stateful failure and
//! cancellation checks live alongside these in lib.rs and dma_checks.rs.
use crate::nice_view::NiceView;
use embedded_hal::digital::OutputPin;
use embedded_hal_mock::eh1::{
    delay::NoopDelay,
    digital::{Mock as PinMock, State, Transaction as PinTransaction},
    spi::{Mock as SpiMock, Transaction as SpiTransaction},
};
use std::convert::Infallible;

// Production pins are infallible. The mock can inject GPIO errors, so fail the
// test immediately if an unexpected operation occurs rather than propagate one.
struct Pin(PinMock);
impl embedded_hal::digital::ErrorType for Pin {
    type Error = Infallible;
}
impl OutputPin for Pin {
    fn set_low(&mut self) -> Result<(), Infallible> {
        self.0.set_low().unwrap();
        Ok(())
    }
    fn set_high(&mut self) -> Result<(), Infallible> {
        self.0.set_high().unwrap();
        Ok(())
    }
}

#[test]
fn panel_sends_clear_and_alternating_hold_commands_with_cs_released() {
    let mut spi = SpiMock::new(&[
        SpiTransaction::write_vec(vec![4, 0]),
        SpiTransaction::flush(),
        SpiTransaction::write_vec(vec![2, 0]),
        SpiTransaction::flush(),
        SpiTransaction::write_vec(vec![0, 0]),
        SpiTransaction::flush(),
    ]);
    let mut cs = PinMock::new(&[
        PinTransaction::set(State::Low),
        PinTransaction::set(State::High),
        PinTransaction::set(State::Low),
        PinTransaction::set(State::High),
        PinTransaction::set(State::Low),
        PinTransaction::set(State::High),
        PinTransaction::set(State::Low),
    ]);
    let mut power = PinMock::new(&[PinTransaction::set(State::High)]);
    let mut panel = NiceView::new(spi.clone(), Pin(cs.clone()), Pin(power.clone()), NoopDelay);
    futures::executor::block_on(async {
        panel.initialize(false).await.unwrap();
        panel.maintain(true).await.unwrap();
        panel.maintain(false).await.unwrap();
    });
    spi.done();
    cs.done();
    power.done();
}
