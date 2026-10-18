//! Screen rail/cadence behavior through the real RMK event subscriptions.

use crate::{
    nice_view::FrameBuffer,
    runtime::{Central, Peripheral, ScreenProcessor, ScreenRole},
    screen::{Role, ScreenState},
    tests::{panel, EVENT_LOCK},
};
use core::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};
use embassy_time::{Duration, MockDriver};
use rmk::{
    event::{publish_event, BatteryStatusEvent, LayerChangeEvent, SleepStateEvent},
    processor::PollingProcessor,
    types::battery::{BatteryStatus, ChargeState},
};

fn poll_pending<F: Future>(future: &mut Pin<Box<F>>, cx: &mut Context<'_>) {
    assert!(matches!(future.as_mut().poll(cx), Poll::Pending));
}

fn sleep_wake_roundtrip<R: ScreenRole>() {
    let driver = MockDriver::get();
    driver.reset();
    let (panel, trace) = panel();
    trace.borrow_mut().mock_clock = true;
    let mut processor = ScreenProcessor::<_, _, _, _, R>::new(panel, || false);
    let mut running = Box::pin(processor.polling_loop());
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    poll_pending(&mut running, &mut cx);
    poll_pending(&mut running, &mut cx);
    driver.advance(Duration::from_micros(16_000));
    poll_pending(&mut running, &mut cx);
    assert_eq!(trace.borrow().writes.len(), 2);

    publish_event(SleepStateEvent::new(true));
    poll_pending(&mut running, &mut cx);
    assert!(!trace.borrow().power, "sleep must cut the display rail");
    assert!(!trace.borrow().cs);
    let sleeping_writes = trace.borrow().writes.len();
    assert_eq!(sleeping_writes, 3, "one final clear, not an extra image");
    assert_eq!(trace.borrow().writes.last().unwrap()[0] & 4, 4);
    {
        let t = trace.borrow();
        let (_, off_at, selected) = t.power_edges.last().copied().unwrap();
        assert!(!selected, "never cut the rail with CS selected");
        assert!(off_at - t.transaction_ends.last().unwrap() >= 30);
    }
    driver.advance(Duration::from_secs(60));
    poll_pending(&mut running, &mut cx);
    assert_eq!(trace.borrow().writes.len(), sleeping_writes);

    publish_event(BatteryStatusEvent(BatteryStatus::Available {
        charge_state: ChargeState::Discharging,
        level: Some(33),
    }));
    publish_event(LayerChangeEvent(2));
    poll_pending(&mut running, &mut cx);
    poll_pending(&mut running, &mut cx);
    assert!(!trace.borrow().power);
    assert_eq!(trace.borrow().writes.len(), sleeping_writes);

    publish_event(SleepStateEvent::new(false));
    poll_pending(&mut running, &mut cx);
    poll_pending(&mut running, &mut cx);
    assert!(trace.borrow().power);
    assert_eq!(trace.borrow().power_levels, [true, false, true]);
    assert_eq!(trace.borrow().writes.len(), sleeping_writes + 1);
    assert_eq!(trace.borrow().writes.last().unwrap(), &[4, 0]);
    assert_eq!(
        trace
            .borrow()
            .delays
            .iter()
            .filter(|&&ns| ns >= 50_000_000)
            .count(),
        2
    );
    {
        let t = trace.borrow();
        let (_, on_at, selected) = t.power_edges.last().copied().unwrap();
        assert!(!selected);
        assert!(t.transaction_starts[sleeping_writes] - on_at >= 50_000);
    }
    driver.advance(Duration::from_micros(16_000));
    poll_pending(&mut running, &mut cx);
    let mut state = ScreenState::new(R::ROLE);
    state.set_battery(Some(33));
    if R::ROLE == Role::Central {
        state.set_layer(2);
    }
    let mut framebuffer = FrameBuffer::new();
    let _ = state.render(&mut framebuffer, 0);
    assert_eq!(
        trace.borrow().writes.last().unwrap(),
        framebuffer.frame(true)
    );
    assert_eq!(trace.borrow().writes.len(), sleeping_writes + 2);
    drop(running);
}

#[test]
fn screen_sleep_powers_off_and_wake_redraws_latest_state_on_both_roles() {
    let _lock = EVENT_LOCK.lock().unwrap();
    sleep_wake_roundtrip::<Central>();
    sleep_wake_roundtrip::<Peripheral>();
}

#[test]
fn screen_sleep_queued_before_startup_never_draws_an_image() {
    let _lock = EVENT_LOCK.lock().unwrap();
    let driver = MockDriver::get();
    driver.reset();
    let (panel, trace) = panel();
    trace.borrow_mut().mock_clock = true;
    let mut processor = ScreenProcessor::<_, _, _, _, Peripheral>::new(panel, || false);
    publish_event(SleepStateEvent::new(true));
    let mut running = Box::pin(processor.polling_loop());
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    poll_pending(&mut running, &mut cx);
    poll_pending(&mut running, &mut cx);
    assert!(!trace.borrow().power);
    assert_eq!(trace.borrow().writes, [vec![4, 0]]);
    for _ in 0..5 {
        driver.advance(Duration::from_secs(60));
        poll_pending(&mut running, &mut cx);
    }
    assert_eq!(trace.borrow().writes.len(), 1);
    assert_eq!(trace.borrow().power_levels, [true, false]);
}

#[test]
fn screen_sleep_has_no_periodic_display_wakeups() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::task::{Wake, Waker};
    struct CountWake(AtomicUsize);
    impl Wake for CountWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let _lock = EVENT_LOCK.lock().unwrap();
    let driver = MockDriver::get();
    driver.reset();
    let (panel, trace) = panel();
    trace.borrow_mut().mock_clock = true;
    let mut processor = ScreenProcessor::<_, _, _, _, Central>::new(panel, || false);
    let count = Arc::new(CountWake(AtomicUsize::new(0)));
    let waker = Waker::from(count.clone());
    let mut cx = Context::from_waker(&waker);
    let mut running = Box::pin(processor.polling_loop());
    poll_pending(&mut running, &mut cx);
    publish_event(SleepStateEvent::new(true));
    poll_pending(&mut running, &mut cx);
    assert!(!trace.borrow().power);
    // A canceled awake timer can leave one stale alarm; consume it first.
    driver.advance(Duration::from_secs(1));
    poll_pending(&mut running, &mut cx);
    count.0.store(0, Ordering::Relaxed);
    let writes = trace.borrow().writes.len();
    for _ in 0..5 {
        driver.advance(Duration::from_secs(60));
        assert_eq!(
            count.0.load(Ordering::Relaxed),
            0,
            "asleep display rearmed a timer"
        );
        poll_pending(&mut running, &mut cx);
    }
    assert_eq!(trace.borrow().writes.len(), writes);
}

#[test]
fn screen_sleep_clear_error_still_cuts_power_and_waits_for_wake() {
    let _lock = EVENT_LOCK.lock().unwrap();
    let driver = MockDriver::get();
    driver.reset();
    let (panel, trace) = panel();
    trace.borrow_mut().mock_clock = true;
    let mut processor = ScreenProcessor::<_, _, _, _, Central>::new(panel, || false);
    let mut running = Box::pin(processor.polling_loop());
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    poll_pending(&mut running, &mut cx);
    poll_pending(&mut running, &mut cx);
    let next_call = trace.borrow().calls + 1;
    trace.borrow_mut().fail = Some(next_call);
    publish_event(SleepStateEvent::new(true));
    poll_pending(&mut running, &mut cx);
    assert!(!trace.borrow().power && !trace.borrow().cs);
    let writes = trace.borrow().writes.len();
    for _ in 0..3 {
        driver.advance(Duration::from_secs(1));
        poll_pending(&mut running, &mut cx);
    }
    assert_eq!(
        trace.borrow().writes.len(),
        writes,
        "do not retry SPI on an unpowered screen"
    );
    trace.borrow_mut().fail = None;
    publish_event(SleepStateEvent::new(false));
    poll_pending(&mut running, &mut cx);
    assert!(trace.borrow().power);
    assert_eq!(trace.borrow().writes.last().unwrap(), &[4, 0]);
    driver.advance(Duration::from_micros(16_000));
    poll_pending(&mut running, &mut cx);
    assert_eq!(trace.borrow().writes.last().unwrap().len(), 1498);
}

#[derive(Default)]
struct Gate {
    blocked: std::cell::Cell<bool>,
    waiter: std::cell::RefCell<Option<std::task::Waker>>,
}

impl Gate {
    fn release(&self) {
        self.blocked.set(false);
        if let Some(waker) = self.waiter.borrow_mut().take() {
            waker.wake();
        }
    }
}

struct GatedBus {
    bus: crate::tests::Bus,
    gate: std::rc::Rc<Gate>,
}

impl embedded_hal::spi::ErrorType for GatedBus {
    type Error = crate::tests::Fault;
}
impl embedded_hal_async::spi::SpiBus for GatedBus {
    async fn write(&mut self, bytes: &[u8]) -> Result<(), Self::Error> {
        futures::future::poll_fn(|cx| {
            if self.gate.blocked.get() {
                *self.gate.waiter.borrow_mut() = Some(cx.waker().clone());
                Poll::Pending
            } else {
                Poll::Ready(())
            }
        })
        .await;
        self.bus.write(bytes).await
    }
    async fn flush(&mut self) -> Result<(), Self::Error> {
        self.bus.flush().await
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
}

fn gated_panel() -> (
    crate::nice_view::NiceView<GatedBus, crate::tests::Pin, crate::tests::Pin, crate::tests::Delay>,
    crate::tests::Shared,
    std::rc::Rc<Gate>,
) {
    let trace = crate::tests::Shared::default();
    trace.borrow_mut().mock_clock = true;
    let gate = std::rc::Rc::new(Gate::default());
    let panel = crate::nice_view::NiceView::new(
        GatedBus {
            bus: crate::tests::Bus(trace.clone()),
            gate: gate.clone(),
        },
        crate::tests::Pin(trace.clone(), false),
        crate::tests::Pin(trace.clone(), true),
        crate::tests::Delay(trace.clone()),
    );
    (panel, trace, gate)
}

#[test]
fn screen_sleep_finishes_in_flight_spi_before_cutting_power() {
    let _lock = EVENT_LOCK.lock().unwrap();
    let driver = MockDriver::get();
    driver.reset();
    let (panel, trace, gate) = gated_panel();
    let mut processor = ScreenProcessor::<_, _, _, _, Central>::new(panel, || false);
    let mut running = Box::pin(processor.polling_loop());
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    poll_pending(&mut running, &mut cx);
    poll_pending(&mut running, &mut cx);
    gate.blocked.set(true);
    driver.advance(Duration::from_micros(16_000));
    poll_pending(&mut running, &mut cx);
    assert!(trace.borrow().cs && trace.borrow().power);
    publish_event(SleepStateEvent::new(true));
    poll_pending(&mut running, &mut cx);
    assert!(
        trace.borrow().cs && trace.borrow().power,
        "do not interrupt active DMA"
    );
    gate.release();
    poll_pending(&mut running, &mut cx);
    assert!(!trace.borrow().cs && !trace.borrow().power);
    assert_eq!(trace.borrow().writes.len(), 3);
    let t = trace.borrow();
    assert!(t
        .power_edges
        .iter()
        .all(|&(high, _, selected)| high || !selected));
}

#[test]
fn screen_sleep_does_not_lose_wake_during_shutdown_clear() {
    let _lock = EVENT_LOCK.lock().unwrap();
    let driver = MockDriver::get();
    driver.reset();
    let (panel, trace, gate) = gated_panel();
    let mut processor = ScreenProcessor::<_, _, _, _, Peripheral>::new(panel, || false);
    let mut running = Box::pin(processor.polling_loop());
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    poll_pending(&mut running, &mut cx);
    poll_pending(&mut running, &mut cx);
    gate.blocked.set(true);
    publish_event(SleepStateEvent::new(true));
    poll_pending(&mut running, &mut cx);
    assert!(trace.borrow().power && trace.borrow().cs);
    publish_event(SleepStateEvent::new(false));
    gate.release();
    poll_pending(&mut running, &mut cx);
    poll_pending(&mut running, &mut cx);
    assert!(trace.borrow().power && !trace.borrow().cs);
    assert_eq!(trace.borrow().power_levels, [true, false, true]);
    assert_eq!(trace.borrow().writes.len(), 3);
    assert_eq!(trace.borrow().writes.last().unwrap(), &[4, 0]);
    driver.advance(Duration::from_micros(16_000));
    poll_pending(&mut running, &mut cx);
    assert_eq!(trace.borrow().writes.last().unwrap().len(), 1498);
}
