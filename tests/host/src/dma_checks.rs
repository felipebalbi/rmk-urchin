//! Active DMA model: retains only the caller's pointer, never an owned copy.
use crate::{
    display_spi::{CancelSafe, DmaControl, SpiFactory, Transient},
    nice_view::NiceView,
};
use embedded_hal::{digital, spi};
use embedded_hal_async::spi::SpiBus;
use std::{
    cell::RefCell,
    convert::Infallible,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll},
};

#[derive(Default)]
struct Dma {
    active: bool,
    end: bool,
    stopping: bool,
    stop_polls: usize,
    pointer: Option<(*const u8, usize)>,
    seen: Vec<u8>,
    log: Vec<&'static str>,
    complete: bool,
    live_buses: usize,
    dropped_buses: usize,
}
type State = Rc<RefCell<Dma>>;
struct Bus(State);
impl Drop for Bus {
    fn drop(&mut self) {
        let mut s = self.0.borrow_mut();
        assert!(!s.active, "peripheral dropped before DMA quiesced");
        s.live_buses -= 1;
        s.dropped_buses += 1;
        s.log.push("bus-drop");
    }
}
impl spi::ErrorType for Bus {
    type Error = Infallible;
}
struct Write<'a> {
    state: State,
    bytes: &'a [u8],
    started: bool,
}
impl Future for Write<'_> {
    type Output = Result<(), Infallible>;
    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        if !self.started {
            let mut s = self.state.borrow_mut();
            assert!(!s.active, "overlapping DMA");
            s.pointer = Some((self.bytes.as_ptr(), self.bytes.len()));
            s.active = true;
            s.end = false;
            s.log.push("start");
            drop(s);
            self.started = true;
        }
        let mut s = self.state.borrow_mut();
        if s.complete {
            s.active = false;
            s.end = true;
            s.log.push("end");
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}
impl Drop for Write<'_> {
    fn drop(&mut self) {
        let mut s = self.state.borrow_mut();
        assert!(
            !s.active,
            "DMA still reading when inner future and its borrowed/copy buffers are dropped"
        );
        s.log.push("future-drop");
        s.pointer = None;
    }
}
impl SpiBus for Bus {
    async fn write(&mut self, b: &[u8]) -> Result<(), Infallible> {
        Write {
            state: self.0.clone(),
            bytes: b,
            started: false,
        }
        .await
    }
    async fn read(&mut self, _: &mut [u8]) -> Result<(), Infallible> {
        unreachable!()
    }
    async fn transfer(&mut self, _: &mut [u8], _: &[u8]) -> Result<(), Infallible> {
        unreachable!()
    }
    async fn transfer_in_place(&mut self, _: &mut [u8]) -> Result<(), Infallible> {
        unreachable!()
    }
    async fn flush(&mut self) -> Result<(), Infallible> {
        assert!(!self.0.borrow().active);
        Ok(())
    }
}
struct Control(State);
impl DmaControl for Control {
    fn disable_end_interrupt(&mut self) {
        self.0.borrow_mut().log.push("mask-end");
    }
    fn ended(&self) -> bool {
        self.0.borrow().end
    }
    fn clear_stopped(&mut self) {
        self.0.borrow_mut().log.push("clear-stopped");
    }
    fn stop(&mut self) {
        let mut s = self.0.borrow_mut();
        s.log.push("stop");
        s.stopping = true;
    }
    fn stopped(&mut self) -> bool {
        let mut s = self.0.borrow_mut();
        assert!(s.stopping);
        s.stop_polls += 1;
        let (p, n) = s.pointer.expect("buffer must remain owned until STOPPED");
        // SAFETY: the test requires the cancellation boundary to retain the
        // borrowed inner future until the modeled DMA has stopped.
        s.seen = unsafe { std::slice::from_raw_parts(p, n) }.to_vec();
        if s.stop_polls < 3 {
            s.log.push("draining-byte");
            false
        } else {
            s.active = false;
            s.end = true;
            s.log.push("stopped");
            true
        }
    }
    fn reset_after_abort(&mut self) {
        let mut s = self.0.borrow_mut();
        assert!(!s.active);
        s.stopping = false;
        s.end = false;
        s.log.push("reset");
    }
}
fn bus(state: &State) -> CancelSafe<Bus, Control> {
    state.borrow_mut().live_buses += 1;
    // SAFETY: both objects control the same modeled DMA, Ready quiesces it,
    // and no external operation starts DMA.
    unsafe { CancelSafe::new(Bus(state.clone()), Control(state.clone())) }
}

struct Factory(State);
impl SpiFactory for Factory {
    type Error = Infallible;
    type Bus<'a>
        = CancelSafe<Bus, Control>
    where
        Self: 'a;

    fn open(&mut self) -> Self::Bus<'_> {
        self.0.borrow_mut().log.push("open");
        bus(&self.0)
    }
}

#[test]
fn transient_spi_opens_only_for_io_and_drops_after_each_completed_write() {
    let state = State::default();
    state.borrow_mut().complete = true;
    let mut spi = Transient::new(Factory(state.clone()));
    futures::executor::block_on(async {
        spi.flush().await.unwrap();
        assert_eq!(state.borrow().live_buses, 0);
        for bytes in [&[0, 0][..], &[2, 0][..]] {
            spi.write(bytes).await.unwrap();
            assert_eq!(
                state.borrow().live_buses,
                0,
                "peripheral retained between panel ticks"
            );
        }
        spi.flush().await.unwrap();
    });
    let s = state.borrow();
    assert_eq!(s.log.iter().filter(|entry| **entry == "open").count(), 2);
    assert_eq!(s.dropped_buses, 2);
}

#[test]
fn transient_spi_cancellation_stops_dma_before_dropping_the_peripheral_and_cs() {
    let state = State::default();
    let mut panel = NiceView::new(
        Transient::new(Factory(state.clone())),
        Cs(state.clone()),
        Power,
        Delay,
    );
    let mut operation = Box::pin(panel.maintain(false));
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    assert!(operation.as_mut().poll(&mut cx).is_pending());
    assert_eq!(state.borrow().live_buses, 1);
    drop(operation);
    {
        let s = state.borrow();
        let stopped = s.log.iter().position(|entry| *entry == "stopped").unwrap();
        let future_drop = s
            .log
            .iter()
            .position(|entry| *entry == "future-drop")
            .unwrap();
        let bus_drop = s.log.iter().position(|entry| *entry == "bus-drop").unwrap();
        let cs_low = s.log.iter().rposition(|entry| *entry == "cs-low").unwrap();
        assert!(stopped < future_drop && future_drop < bus_drop && bus_drop < cs_low);
        assert_eq!(s.live_buses, 0);
    }
    state.borrow_mut().complete = true;
    futures::executor::block_on(panel.flush(true)).unwrap();
    assert_eq!(state.borrow().live_buses, 0);
    assert_eq!(state.borrow().dropped_buses, 2);
}

struct FallibleFactory(State);
struct FallibleBus(CancelSafe<Bus, Control>);
impl spi::ErrorType for FallibleBus {
    type Error = spi::ErrorKind;
}
impl SpiBus for FallibleBus {
    async fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        self.0.write(bytes).await.unwrap();
        Err(spi::ErrorKind::Other)
    }
    async fn read(&mut self, _: &mut [u8]) -> Result<(), Self::Error> {
        unreachable!()
    }
    async fn transfer(&mut self, _: &mut [u8], _: &[u8]) -> Result<(), Self::Error> {
        unreachable!()
    }
    async fn transfer_in_place(&mut self, _: &mut [u8]) -> Result<(), Self::Error> {
        unreachable!()
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        unreachable!("failed operation must not flush")
    }
}
impl SpiFactory for FallibleFactory {
    type Error = spi::ErrorKind;
    type Bus<'a>
        = FallibleBus
    where
        Self: 'a;
    fn open(&mut self) -> Self::Bus<'_> {
        FallibleBus(bus(&self.0))
    }
}

#[test]
fn transient_spi_drops_the_peripheral_when_an_operation_returns_an_error() {
    let state = State::default();
    state.borrow_mut().complete = true;
    let mut spi = Transient::new(FallibleFactory(state.clone()));
    assert_eq!(
        futures::executor::block_on(spi.write(&[0, 0])),
        Err(spi::ErrorKind::Other)
    );
    assert_eq!(state.borrow().live_buses, 0);
    assert_eq!(state.borrow().dropped_buses, 1);
}
struct Cs(State);
impl digital::ErrorType for Cs {
    type Error = Infallible;
}
impl digital::OutputPin for Cs {
    fn set_low(&mut self) -> Result<(), Infallible> {
        self.0.borrow_mut().log.push("cs-low");
        Ok(())
    }
    fn set_high(&mut self) -> Result<(), Infallible> {
        self.0.borrow_mut().log.push("cs-high");
        Ok(())
    }
}
struct Power;
impl digital::ErrorType for Power {
    type Error = Infallible;
}
impl digital::OutputPin for Power {
    fn set_low(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
    fn set_high(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}
struct Delay;
impl embedded_hal_async::delay::DelayNs for Delay {
    async fn delay_ns(&mut self, _: u32) {}
}

#[test]
fn abort_quiesces_live_dma_before_inner_future_buffer_and_cs_cleanup_then_reuses_bus() {
    for frame in [false, true] {
        let state = State::default();
        let mut panel = NiceView::new(bus(&state), Cs(state.clone()), Power, Delay);
        let mut operation: Pin<Box<dyn Future<Output = Result<(), Infallible>> + '_>> = if frame {
            Box::pin(panel.flush(true))
        } else {
            Box::pin(panel.maintain(true))
        };
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        assert_eq!(operation.as_mut().poll(&mut cx), Poll::Pending);
        assert!(state.borrow().active);
        drop(operation);
        let s = state.borrow();
        assert!(!s.active);
        assert_eq!(
            &s.log[s.log.len() - 9..],
            [
                "mask-end",
                "clear-stopped",
                "stop",
                "draining-byte",
                "draining-byte",
                "stopped",
                "reset",
                "future-drop",
                "cs-low"
            ]
        );
        // Check exact lifecycle order without copying away the caller's DMA ownership.
        let stop = s.log.iter().position(|e| *e == "stop").unwrap();
        let stopped = s.log.iter().position(|e| *e == "stopped").unwrap();
        let dropped = s.log.iter().position(|e| *e == "future-drop").unwrap();
        assert!(stop < stopped && stopped < dropped);
        assert_eq!(s.log.last(), Some(&"cs-low"));
        assert_eq!(s.seen[0], if frame { 3 } else { 2 });
        drop(s);
        // Immediate mutation/retry is safe; no timeout/sleep hides a still-running DMA.
        panel.framebuffer().frame(false);
        state.borrow_mut().complete = true;
        futures::executor::block_on(panel.flush(false)).unwrap();
        assert!(!state.borrow().active);
    }
}

#[test]
fn completed_but_unpolled_dma_is_quiescent_and_does_not_wait_for_an_unneeded_stop() {
    let state = State::default();
    let mut bus = bus(&state);
    let bytes = [2, 0];
    let mut write = Box::pin(bus.write(&bytes));
    assert_eq!(
        write
            .as_mut()
            .poll(&mut Context::from_waker(futures::task::noop_waker_ref())),
        Poll::Pending
    );
    {
        let mut s = state.borrow_mut();
        s.active = false;
        s.end = true;
        s.log.push("hardware-end");
    }
    drop(write);
    let s = state.borrow();
    assert!(!s.log.contains(&"stop"));
    assert_eq!(
        &s.log[s.log.len() - 3..],
        ["mask-end", "reset", "future-drop"]
    );
}
