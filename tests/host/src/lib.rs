//! Host checks compile the actual firmware modules against crates.io RMK.

#![cfg(test)]

#[cfg(test)]
mod debounce_checks;
#[path = "../../../src/display_spi.rs"]
#[cfg(test)]
mod display_spi;
#[cfg(test)]
mod dma_checks;
#[path = "../../../src/nice_view.rs"]
mod nice_view;
#[path = "../../../src/screen/runtime.rs"]
mod runtime;
#[path = "../../../src/screen.rs"]
mod screen;
#[cfg(test)]
mod transport_checks;

#[cfg(test)]
mod tests {
    use super::{nice_view::*, screen::*};
    use embedded_graphics::{pixelcolor::BinaryColor, prelude::*};
    use embedded_hal::{digital, spi};
    use std::{cell::RefCell, convert::Infallible, rc::Rc};
    pub(crate) static EVENT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static LEFT_USB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static RIGHT_USB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    fn left_usb() -> bool {
        LEFT_USB.load(std::sync::atomic::Ordering::Relaxed)
    }
    fn right_usb() -> bool {
        RIGHT_USB.load(std::sync::atomic::Ordering::Relaxed)
    }

    #[test]
    fn framebuffer_rotates_clips_and_preserves_every_edge() {
        let mut fb = FrameBuffer::new();
        assert_eq!(fb.size(), Size::new(68, 160));
        let points = [
            (0, 0),
            (67, 0),
            (0, 159),
            (67, 159),
            (1, 0),
            (66, 159),
            (0, 1),
            (67, 158),
        ];
        fb.draw_iter(
            points
                .into_iter()
                .map(|(x, y)| Pixel(Point::new(x, y), BinaryColor::Off))
                .chain(
                    [(-1, 0), (68, 0), (0, -1), (0, 160), (i32::MAX, i32::MIN)]
                        .into_iter()
                        .map(|(x, y)| Pixel(Point::new(x, y), BinaryColor::Off)),
                ),
        )
        .unwrap();
        let bytes = fb.frame(false);
        assert_eq!(bytes.len(), 1498);
        assert_eq!(bytes[0], 0x01);
        assert_eq!(bytes[1497], 0);
        let mut black = Vec::new();
        for row in 0..68 {
            let start = 1 + row * 22;
            assert_eq!(bytes[start], row as u8 + 1);
            assert_eq!(bytes[start + 21], 0);
            for x in 0..160 {
                if bytes[start + 1 + x / 8] & (1 << (x % 8)) == 0 {
                    black.push((x, row));
                }
            }
        }
        black.sort();
        let mut expected = vec![
            (159, 0),
            (159, 67),
            (0, 0),
            (0, 67),
            (159, 1),
            (0, 66),
            (158, 0),
            (1, 67),
        ];
        expected.sort();
        assert_eq!(black, expected);
        assert_eq!(fb.frame(true)[0], 0x03);
        fb.clear(BinaryColor::On).unwrap();
        assert!(fb.frame(false)[2..22].iter().all(|b| *b == 255));
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    pub enum Fault {
        Bus,
    }
    impl spi::Error for Fault {
        fn kind(&self) -> spi::ErrorKind {
            spi::ErrorKind::Other
        }
    }
    #[derive(Default)]
    pub struct Trace {
        pub cs: bool,
        pub power: bool,
        pub writes: Vec<Vec<u8>>,
        pub accepted: Vec<Vec<u8>>,
        pub power_levels: Vec<bool>,
        pub edges: Vec<bool>,
        pub calls: usize,
        pub fail: Option<usize>,
        pub pending: bool,
        pub delays: Vec<u32>,
        pub now_us: u64,
        pub realistic: bool,
        pub transaction_starts: Vec<u64>,
        pub transaction_ends: Vec<u64>,
        pub fail_after_bytes: Option<usize>,
        pub mock_clock: bool,
    }
    pub type Shared = Rc<RefCell<Trace>>;
    pub struct Bus(pub Shared);
    impl spi::ErrorType for Bus {
        type Error = Fault;
    }
    impl embedded_hal_async::spi::SpiBus for Bus {
        async fn read(&mut self, _: &mut [u8]) -> Result<(), Fault> {
            unreachable!()
        }
        async fn write(&mut self, bytes: &[u8]) -> Result<(), Fault> {
            {
                let mut t = self.0.borrow_mut();
                assert!(t.cs);
                t.calls += 1;
                if let Some(n) = t.fail_after_bytes {
                    t.writes.push(bytes[..n.min(bytes.len())].to_vec());
                    return Err(Fault::Bus);
                }
                t.writes.push(bytes.to_vec());
                if t.realistic {
                    t.now_us += bytes.len() as u64 * 8;
                }
                if t.mock_clock {
                    embassy_time::MockDriver::get()
                        .advance(embassy_time::Duration::from_micros(bytes.len() as u64 * 8));
                }
                if t.fail == Some(t.calls) {
                    return Err(Fault::Bus);
                }
                // SPI has no panel acknowledgement: an unpowered display
                // silently ignores otherwise successful bus transactions.
                if t.power && !t.pending {
                    t.accepted.push(bytes.to_vec());
                }
            }
            if self.0.borrow().pending {
                futures::future::pending::<()>().await;
            }
            Ok(())
        }
        async fn transfer(&mut self, _: &mut [u8], _: &[u8]) -> Result<(), Fault> {
            unreachable!()
        }
        async fn transfer_in_place(&mut self, _: &mut [u8]) -> Result<(), Fault> {
            unreachable!()
        }
        async fn flush(&mut self) -> Result<(), Fault> {
            let mut t = self.0.borrow_mut();
            assert!(t.cs);
            t.calls += 1;
            if t.fail == Some(t.calls) {
                Err(Fault::Bus)
            } else {
                Ok(())
            }
        }
    }
    pub struct Pin(pub Shared, pub bool);
    impl digital::ErrorType for Pin {
        type Error = Infallible;
    }
    impl digital::OutputPin for Pin {
        fn set_low(&mut self) -> Result<(), Infallible> {
            self.set(false);
            Ok(())
        }
        fn set_high(&mut self) -> Result<(), Infallible> {
            self.set(true);
            Ok(())
        }
    }
    impl Pin {
        fn set(&mut self, high: bool) {
            let mut t = self.0.borrow_mut();
            if self.1 {
                t.power = high;
                t.power_levels.push(high);
            } else {
                let now = if t.mock_clock {
                    embassy_time::Instant::now().as_micros()
                } else {
                    t.now_us
                };
                if high {
                    t.transaction_starts.push(now);
                } else if t.cs {
                    t.transaction_ends.push(now);
                }
                t.cs = high;
                t.edges.push(high);
            }
        }
    }
    pub struct Delay(pub Shared);
    impl embedded_hal_async::delay::DelayNs for Delay {
        async fn delay_ns(&mut self, ns: u32) {
            let mut t = self.0.borrow_mut();
            t.delays.push(ns);
            if t.realistic {
                t.now_us += (ns as u64 * 32768).div_ceil(1_000_000_000) * 1_000_000 / 32768;
            }
            if t.mock_clock {
                embassy_time::MockDriver::get()
                    .advance(embassy_time::Duration::from_nanos(ns as u64));
            }
        }
    }
    pub fn panel() -> (NiceView<Bus, Pin, Pin, Delay>, Shared) {
        let t = Shared::default();
        (
            NiceView::new(
                Bus(t.clone()),
                Pin(t.clone(), false),
                Pin(t.clone(), true),
                Delay(t.clone()),
            ),
            t,
        )
    }

    #[test]
    fn nice_nano_v2_supply_powers_startup_image_and_idle_maintenance_on_both_roles() {
        for role in [Role::Central, Role::Peripheral] {
            let (panel, trace) = panel();
            let mut screen = Screen::new(panel, role);
            futures::executor::block_on(async {
                screen.tick(0).await.unwrap();
                screen.tick(16_000).await.unwrap();
                screen.tick(32_000).await.unwrap();
            });
            {
                let t = trace.borrow();
                assert_eq!(
                    t.accepted.len(),
                    3,
                    "{role:?}: unpowered panel ignored successful SPI writes"
                );
                assert_eq!(t.accepted[0], [4, 0]);
                assert_eq!(t.accepted[1].len(), 1498);
                assert_eq!(t.accepted[2], [0, 0]);
                assert_eq!(t.power_levels, [true]);
                assert!(t.power && !t.cs);
                assert!(t.delays.iter().any(|ns| *ns >= 50_000_000));
            }
            // Negative control: a cut v2 supply causes missing panel acceptance,
            // not a fabricated SPI error (the hardware is write-only).
            trace.borrow_mut().power = false;
            futures::executor::block_on(screen.tick(48_000)).unwrap();
            assert_eq!(trace.borrow().writes.len(), 4);
            assert_eq!(trace.borrow().accepted.len(), 3);
        }
    }

    #[test]
    fn panel_clear_maintenance_and_full_redraw_have_valid_framing() {
        let (mut panel, t) = panel();
        futures::executor::block_on(async {
            panel.initialize(false).await.unwrap();
            panel.flush(true).await.unwrap();
            panel.maintain(false).await.unwrap();
        });
        let t = t.borrow();
        assert!(!t.cs && t.power);
        assert_eq!(t.writes[0], [4, 0]);
        assert_eq!(t.writes[1].len(), 1498);
        assert_eq!(t.writes[1][0], 3);
        assert_eq!(t.writes[2], [0, 0]);
        assert!(t.delays.iter().any(|ns| *ns >= 30_000_000));
    }

    #[test]
    fn transfer_errors_release_cs_and_allow_complete_retry() {
        for fail in 1..=6 {
            let (mut p, t) = panel();
            t.borrow_mut().fail = Some(fail);
            let result = futures::executor::block_on(async {
                p.initialize(false).await?;
                p.flush(false).await?;
                p.maintain(true).await
            });
            assert_eq!(result, Err(Fault::Bus));
            assert!(!t.borrow().cs);
            t.borrow_mut().fail = None;
            futures::executor::block_on(p.flush(true)).unwrap();
            let t = t.borrow();
            assert!(!t.cs);
            assert_eq!(t.writes.last().unwrap().len(), 1498);
            assert_eq!(t.writes.last().unwrap()[1497], 0);
        }
    }

    #[test]
    fn dropping_pending_transfer_releases_active_high_cs() {
        use std::{
            future::Future,
            task::{Context, Poll},
        };
        let (mut p, t) = panel();
        t.borrow_mut().pending = true;
        let mut future = Box::pin(p.flush(false));
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        assert_eq!(future.as_mut().poll(&mut cx), Poll::Pending);
        assert!(t.borrow().cs);
        drop(future);
        assert!(!t.borrow().cs);
    }

    fn image(state: &ScreenState) -> Vec<(i32, i32)> {
        struct Canvas(std::collections::BTreeSet<(i32, i32)>);
        impl OriginDimensions for Canvas {
            fn size(&self) -> Size {
                Size::new(68, 160)
            }
        }
        impl DrawTarget for Canvas {
            type Color = BinaryColor;
            type Error = Infallible;
            fn draw_iter<I: IntoIterator<Item = Pixel<BinaryColor>>>(
                &mut self,
                pixels: I,
            ) -> Result<(), Infallible> {
                for Pixel(p, c) in pixels {
                    assert!(
                        (0..68).contains(&p.x) && (0..160).contains(&p.y),
                        "out of bounds: {p:?}"
                    );
                    if c == BinaryColor::On {
                        self.0.insert((p.x, p.y));
                    } else {
                        self.0.remove(&(p.x, p.y));
                    }
                }
                Ok(())
            }
        }
        let mut c = Canvas(Default::default());
        state.render(&mut c, 0).unwrap();
        c.0.into_iter().collect()
    }

    #[test]
    fn role_state_unknown_battery_clamping_labels_and_role_filtering() {
        let mut c = ScreenState::new(Role::Central);
        let mut p = ScreenState::new(Role::Peripheral);
        assert_eq!(c.battery_text().as_str(), "--%");
        assert!(!c.set_battery(None));
        assert!(c.set_battery(Some(0)));
        assert_eq!(c.battery_text().as_str(), "0%");
        assert!(c.set_battery(Some(255)));
        assert_eq!(c.battery_text().as_str(), "100%");
        assert!(!c.set_battery(Some(100)));
        assert_eq!(p.battery_text().as_str(), "--%");
        assert!(p.set_battery(Some(47)));
        assert!(p.set_battery(None));
        for (layer, label) in [
            (0, "Base"),
            (1, "Left"),
            (2, "Right"),
            (3, "Tri"),
            (255, "Layer 255"),
        ] {
            c.set_layer(layer);
            assert_eq!(c.layer_text().as_str(), label);
        }
        let before = image(&p);
        assert!(!p.set_layer(3));
        assert!(!p.set_host(5, HostLink::Connected));
        assert_eq!(image(&p), before);
        let before = image(&c);
        assert!(!c.set_central_connected(true));
        assert_eq!(image(&c), before);
        assert!(p.set_central_connected(true));
        assert_ne!(image(&p), before);
        assert!(!p.set_central_connected(true));
    }

    #[test]
    fn bluetooth_icons_replace_link_text_on_both_roles_and_preserve_profiles() {
        let mut c = ScreenState::new(Role::Central);
        let mut p = ScreenState::new(Role::Peripheral);
        c.set_host(5, HostLink::Connected);
        p.set_central_connected(true);
        for state in [&c, &p] {
            let pixels = image(state);
            assert!(
                pixels.iter().any(|(x, y)| *x >= 50 && (39..54).contains(y)),
                "Bluetooth icon must be next to the role label"
            );
            assert!(
                !pixels.iter().any(|(_, y)| (66..74).contains(y)),
                "link status text must be replaced, not duplicated"
            );
            // Stem and upper loop of the standard Bluetooth rune.
            for point in [(56, 40), (56, 46), (56, 52), (61, 44)] {
                assert!(
                    pixels.contains(&point),
                    "missing Bluetooth rune pixel {point:?}"
                );
            }
        }
        let connected_icon: Vec<_> = image(&c)
            .into_iter()
            .filter(|(x, y)| *x >= 50 && (39..54).contains(y))
            .collect();
        assert_eq!(
            connected_icon,
            image(&p)
                .into_iter()
                .filter(|(x, y)| *x >= 50 && (39..54).contains(y))
                .collect::<Vec<_>>()
        );
        let profile_five = image(&c);
        c.set_host(0, HostLink::Connected);
        assert_ne!(image(&c), profile_five, "profile number remains visible");
        c.set_host(0, HostLink::Inactive);
        p.set_central_connected(false);
        for state in [&c, &p] {
            let pixels = image(state);
            for point in [(60, 46), (66, 46), (63, 49), (60, 52), (66, 52)] {
                assert!(
                    pixels.contains(&point),
                    "missing disconnected X pixel {point:?}"
                );
            }
            assert!(!connected_icon.contains(&(66, 52)));
        }
    }

    fn wire_pixel(wire: &[u8], x: usize, y: usize) -> bool {
        assert_eq!(
            wire.len(),
            1498,
            "expected a complete image, not a hold command"
        );
        let physical_x = 159 - y;
        wire[2 + x * 22 + physical_x / 8] & (1 << (physical_x % 8)) != 0
    }

    #[test]
    fn advertising_blinks_every_half_second_without_events_or_extra_idle_images() {
        let (p, t) = panel();
        let mut screen = Screen::new(p, Role::Central);
        screen.state_mut().set_host(3, HostLink::Advertising);
        futures::executor::block_on(async {
            for now in (0..=1_024_000).step_by(16_000) {
                screen.tick(now).await.unwrap();
            }
        });
        let trace = t.borrow();
        let images: Vec<_> = trace.writes.iter().filter(|w| w.len() == 1498).collect();
        assert_eq!(images.len(), 3, "initial image, blink off, blink on");
        assert!(wire_pixel(images[0], 56, 46));
        assert!(!wire_pixel(images[1], 56, 46));
        assert_eq!(&images[0][1..], &images[2][1..]);
        for x in 0..68 {
            for y in 0..160 {
                if !(x >= 50 && (39..54).contains(&y)) {
                    assert_eq!(
                        wire_pixel(images[0], x, y),
                        wire_pixel(images[1], x, y),
                        "only the icon should blink, not the label/profile"
                    );
                }
            }
        }
        for (i, write) in trace.writes.iter().enumerate() {
            assert_eq!(write[0] & 2, if i % 2 == 0 { 0 } else { 2 });
        }
    }

    #[test]
    fn advertising_to_connected_restores_steady_icon_and_stops_animation() {
        let (p, t) = panel();
        let mut screen = Screen::new(p, Role::Central);
        screen.state_mut().set_host(0, HostLink::Advertising);
        futures::executor::block_on(async {
            screen.tick(0).await.unwrap();
            screen.tick(16_000).await.unwrap();
            screen.tick(512_000).await.unwrap();
        });
        assert!(!wire_pixel(t.borrow().writes.last().unwrap(), 56, 46));
        screen.state_mut().set_host(0, HostLink::Connected);
        futures::executor::block_on(screen.tick(624_000)).unwrap();
        assert!(wire_pixel(t.borrow().writes.last().unwrap(), 56, 46));
        let images = t.borrow().writes.iter().filter(|w| w.len() == 1498).count();
        for now in [1_008_000, 1_504_000, 2_000_000] {
            futures::executor::block_on(screen.tick(now)).unwrap();
        }
        assert_eq!(
            t.borrow().writes.iter().filter(|w| w.len() == 1498).count(),
            images
        );
    }

    #[test]
    fn fixed_ticks_coalesce_changes_and_invert_while_idle_without_full_redraws() {
        let (p, t) = panel();
        let mut screen = Screen::new(p, Role::Central);
        futures::executor::block_on(async {
            screen.tick(0).await.unwrap();
            for now in (16_000..=3_200_000).step_by(16_000) {
                screen.tick(now).await.unwrap();
            }
        });
        let trace = t.borrow();
        assert_eq!(trace.writes.iter().filter(|w| w.len() == 1498).count(), 1);
        let commands: Vec<_> = trace
            .writes
            .iter()
            .filter(|w| w.len() == 2)
            .map(|w| w[0])
            .collect();
        assert_eq!(commands.len(), 200); // clear, 199 hold commands; first image occupies one tick
        for (i, w) in trace.writes.iter().enumerate() {
            assert_eq!(w[0] & 2, if i % 2 == 0 { 0 } else { 2 });
        }
        drop(trace);
        for i in 0..30 {
            screen
                .state_mut()
                .set_host((i % 6) as u8, HostLink::Connected);
            screen.state_mut().set_layer((i % 4) as u8);
        }
        futures::executor::block_on(screen.tick(3_216_000)).unwrap();
        let mut expected = FrameBuffer::new();
        let mut state = ScreenState::new(Role::Central);
        state.set_host(5, HostLink::Connected);
        state.set_layer(1);
        state.render(&mut expected, 0).unwrap();
        assert_eq!(t.borrow().writes.last().unwrap(), expected.frame(true));
        // Content writes near the next deadline must not defer inversion.
        screen.state_mut().set_battery(Some(42));
        futures::executor::block_on(screen.tick(3_232_000)).unwrap();
        assert_eq!(t.borrow().writes.last().unwrap(), &vec![0, 0]);
        for now in (3_248_000..=3_328_000).step_by(16_000) {
            futures::executor::block_on(screen.tick(now)).unwrap();
        }
        assert_eq!(t.borrow().writes.last().unwrap().len(), 1498);
    }

    #[test]
    fn initialization_and_maintenance_failures_leave_full_redraw_pending() {
        for fail in [1, 2, 3, 4, 5, 6] {
            let (p, t) = panel();
            let mut s = Screen::new(p, Role::Peripheral);
            t.borrow_mut().fail = Some(fail);
            let mut failed = false;
            futures::executor::block_on(async {
                for now in (0..=128_000).step_by(16_000) {
                    if s.tick(now).await.is_err() {
                        failed = true;
                        break;
                    }
                }
                assert!(failed);
                assert!(!t.borrow().cs);
                t.borrow_mut().fail = None;
                s.tick(256_000).await.unwrap();
                // Initialization retry itself sends a clear, image is next tick.
                if fail <= 2 {
                    s.tick(272_000).await.unwrap();
                }
            });
            assert_eq!(t.borrow().writes.last().unwrap().len(), 1498);
        }
    }

    #[test]
    fn startup_subscriptions_capture_real_rmk_events_and_converge_after_bursts() {
        let _lock = EVENT_LOCK.lock().unwrap();
        use crate::runtime::Subscriptions;
        use rmk::{
            event::{
                publish_event, BatteryStatusEvent, CentralConnectedEvent, ConnectionStatus,
                ConnectionStatusChangeEvent, LayerChangeEvent,
            },
            types::{
                battery::{BatteryStatus, ChargeState},
                ble::BleState,
            },
        };
        let mut central = Subscriptions::new(Role::Central);
        let mut peripheral = Subscriptions::new(Role::Peripheral);
        let mut c = ScreenState::new(Role::Central);
        let mut p = ScreenState::new(Role::Peripheral);
        publish_event(BatteryStatusEvent(BatteryStatus::Available {
            charge_state: ChargeState::Discharging,
            level: Some(73),
        }));
        publish_event(CentralConnectedEvent { connected: true });
        let mut connection = ConnectionStatus::new();
        connection.ble.profile = 5;
        connection.ble.state = BleState::Advertising;
        publish_event(ConnectionStatusChangeEvent(connection));
        publish_event(LayerChangeEvent(3));
        central.drain_into(&mut c);
        peripheral.drain_into(&mut p);
        let mut expected = ScreenState::new(Role::Central);
        expected.set_battery(Some(73));
        expected.set_host(5, HostLink::Advertising);
        expected.set_layer(3);
        assert_eq!(c, expected);
        let mut expected_p = ScreenState::new(Role::Peripheral);
        expected_p.set_battery(Some(73));
        expected_p.set_central_connected(true);
        assert_eq!(p, expected_p);
        for i in 0..30 {
            connection.ble.profile = (i % 6) as u8;
            connection.ble.state = BleState::Connected;
            publish_event(ConnectionStatusChangeEvent(connection));
            publish_event(LayerChangeEvent((i % 4) as u8));
        }
        central.drain_into(&mut c);
        expected.set_host(5, HostLink::Connected);
        expected.set_layer(1);
        assert_eq!(c, expected);
        publish_event(BatteryStatusEvent(BatteryStatus::Unavailable));
        central.drain_into(&mut c);
        assert_eq!(c.battery_text().as_str(), "--%");
        // A separate instance has not consumed this later local measurement yet.
        assert_eq!(p.battery_text().as_str(), "73%");
        peripheral.drain_into(&mut p);
        assert_eq!(p.battery_text().as_str(), "--%");
    }

    #[test]
    fn real_transfer_duration_keeps_57_to_70_hz_without_image_or_delay_drift() {
        let (p, t) = panel();
        let mut s = Screen::new(p, Role::Central);
        t.borrow_mut().realistic = true;
        futures::executor::block_on(s.settle());
        let now = t.borrow().now_us;
        let mut cadence = PanelCadence::new(now);
        for i in 0..250 {
            t.borrow_mut().now_us = cadence.deadline_us();
            let start = t.borrow().now_us;
            s.state_mut().set_layer((i % 4) as u8);
            futures::executor::block_on(s.tick(start)).unwrap();
            cadence.completed(start, t.borrow().now_us);
        }
        let t = t.borrow();
        assert_eq!(t.transaction_starts.len(), 250);
        for gap in t.transaction_starts.windows(2) {
            assert!((14_290..=17_540).contains(&(gap[1] - gap[0])), "{gap:?}");
        }
        let mut last_image = None;
        for (i, write) in t.writes.iter().enumerate() {
            assert_eq!(write[0] & 2, if i % 2 == 0 { 0 } else { 2 });
            let duration = t.transaction_ends[i] - t.transaction_starts[i];
            if write.len() == 1498 {
                assert!((11_984..16_000).contains(&duration));
                if let Some(last) = last_image {
                    assert!(t.transaction_starts[i] - last >= 100_000);
                }
                last_image = Some(t.transaction_starts[i]);
            } else {
                assert!(duration >= 23);
            }
        }
        assert!(last_image.is_some());
        // An overrun skips stale deadlines rather than issuing a short burst.
        let mut cadence = PanelCadence::new(0);
        cadence.completed(0, 20_000);
        assert_eq!(cadence.deadline_us(), 32_000);
        cadence.completed(32_000, 35_000);
        assert_eq!(cadence.deadline_us(), 48_000);
    }

    #[test]
    fn partially_transmitted_frames_release_cs_and_recover_entire_image() {
        for prefix in [0, 1, 749, 1497, 1498] {
            let (mut p, t) = panel();
            p.framebuffer()
                .draw_iter([Pixel(Point::new(67, 159), BinaryColor::Off)])
                .unwrap();
            t.borrow_mut().fail_after_bytes = Some(prefix);
            assert_eq!(futures::executor::block_on(p.flush(false)), Err(Fault::Bus));
            assert!(!t.borrow().cs);
            t.borrow_mut().fail_after_bytes = None;
            futures::executor::block_on(p.flush(true)).unwrap();
            let wire = t.borrow().writes.last().unwrap().clone();
            assert_eq!(wire.len(), 1498);
            assert_eq!(wire[0], 3);
            assert_eq!(wire[1476], 254);
            assert_eq!(wire[1497], 0);
        }
    }

    #[test]
    fn cancelled_maintenance_keeps_redraw_pending_and_local_batteries_stay_separate() {
        use std::{
            future::Future,
            task::{Context, Poll},
        };
        let (p, t) = panel();
        let mut s = Screen::new(p, Role::Central);
        let (p2, t2) = panel();
        let mut s2 = Screen::new(p2, Role::Peripheral);
        s.state_mut().set_battery(Some(20));
        s2.state_mut().set_battery(Some(83));
        futures::executor::block_on(async {
            s.tick(0).await.unwrap();
            s.tick(16_000).await.unwrap();
            s2.tick(0).await.unwrap();
            s2.tick(16_000).await.unwrap();
        });
        assert_ne!(
            t.borrow().writes.last().unwrap(),
            t2.borrow().writes.last().unwrap()
        );
        t.borrow_mut().pending = true;
        let mut future = Box::pin(s.tick(128_000));
        assert_eq!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(futures::task::noop_waker_ref())),
            Poll::Pending
        );
        assert!(t.borrow().cs);
        drop(future);
        assert!(!t.borrow().cs);
        t.borrow_mut().pending = false;
        futures::executor::block_on(s.tick(144_000)).unwrap();
        assert_eq!(t.borrow().writes.last().unwrap().len(), 1498);
    }

    #[test]
    fn actual_registered_polling_loop_retains_startup_events_and_fixed_cadence() {
        use crate::runtime::{Central, ScreenProcessor};
        use rmk::{
            event::{publish_event, BatteryStatusEvent, LayerChangeEvent},
            processor::PollingProcessor,
            types::battery::{BatteryStatus, ChargeState},
        };
        use std::{
            future::Future,
            task::{Context, Poll},
        };
        let _lock = EVENT_LOCK.lock().unwrap();
        let driver = embassy_time::MockDriver::get();
        driver.reset();
        let (panel, t) = panel();
        t.borrow_mut().mock_clock = true;
        let mut processor = ScreenProcessor::<_, _, _, _, Central>::new(panel, || false);
        publish_event(BatteryStatusEvent(BatteryStatus::Available {
            charge_state: ChargeState::Discharging,
            level: Some(61),
        }));
        publish_event(LayerChangeEvent(3));
        let mut running = Box::pin(processor.polling_loop());
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        // Timer::at deliberately yields on its first poll even if already due.
        assert_eq!(running.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(running.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(t.borrow().writes.len(), 1);
        for i in 1..200 {
            let target = t.borrow().transaction_starts[i - 1] + 16_000;
            driver.advance(embassy_time::Duration::from_micros(
                target.saturating_sub(embassy_time::Instant::now().as_micros()),
            ));
            assert_eq!(running.as_mut().poll(&mut cx), Poll::Pending);
            assert_eq!(t.borrow().writes.len(), i + 1);
        }
        drop(running);
        let trace = t.borrow();
        assert_eq!(trace.writes.iter().filter(|w| w.len() == 1498).count(), 1);
        for gap in trace.transaction_starts.windows(2) {
            assert!((14_290..=17_540).contains(&(gap[1] - gap[0])), "{gap:?}");
        }
        let mut fb = FrameBuffer::new();
        let mut expected = ScreenState::new(Role::Central);
        expected.set_battery(Some(61));
        expected.set_layer(3);
        expected.render(&mut fb, 0).unwrap();
        assert_eq!(&trace.writes[1], fb.frame(true));
        assert!(!trace.cs);
    }

    #[test]
    fn failed_transaction_preserves_last_success_and_alternates_attempted_phase() {
        let (p, t) = panel();
        let mut s = Screen::new(p, Role::Central);
        assert_eq!(s.last_success(), None);
        futures::executor::block_on(s.tick(0)).unwrap();
        assert_eq!(s.last_success(), Some((0, false)));
        t.borrow_mut().fail = Some(3);
        assert_eq!(futures::executor::block_on(s.tick(16_000)), Err(Fault::Bus));
        assert_eq!(s.last_success(), Some((0, false)));
        t.borrow_mut().fail = None;
        futures::executor::block_on(s.tick(32_000)).unwrap();
        assert_eq!(s.last_success(), Some((32_000, false)));
        futures::executor::block_on(s.tick(48_000)).unwrap();
        assert_eq!(s.last_success(), Some((48_000, true)));
        assert_eq!(
            t.borrow()
                .writes
                .iter()
                .map(|w| w[0] & 2)
                .collect::<Vec<_>>(),
            vec![0, 2, 0, 2]
        );
        let (p, t) = panel();
        let mut s = Screen::new(p, Role::Peripheral);
        t.borrow_mut().fail = Some(1);
        assert!(futures::executor::block_on(s.tick(0)).is_err());
        t.borrow_mut().fail = None;
        futures::executor::block_on(s.tick(16_000)).unwrap();
        assert_eq!(t.borrow().writes.last().unwrap(), &vec![6, 0]);
    }

    #[test]
    fn peripheral_marker_selects_local_link_and_never_host_profile_or_layer() {
        use crate::runtime::{Peripheral, ScreenProcessor};
        use rmk::{
            event::{
                publish_event, BatteryStatusEvent, CentralConnectedEvent, ConnectionStatus,
                ConnectionStatusChangeEvent, LayerChangeEvent,
            },
            processor::PollingProcessor,
            types::{
                battery::{BatteryStatus, ChargeState},
                ble::BleState,
            },
        };
        use std::{
            future::Future,
            task::{Context, Poll},
        };
        let _lock = EVENT_LOCK.lock().unwrap();
        let driver = embassy_time::MockDriver::get();
        driver.reset();
        let (p, t) = panel();
        t.borrow_mut().mock_clock = true;
        let mut processor = ScreenProcessor::<_, _, _, _, Peripheral>::new(p, || false);
        publish_event(BatteryStatusEvent(BatteryStatus::Available {
            charge_state: ChargeState::Unknown,
            level: Some(42),
        }));
        publish_event(CentralConnectedEvent { connected: true });
        publish_event(LayerChangeEvent(3));
        let mut connection = ConnectionStatus::new();
        connection.ble.profile = 5;
        connection.ble.state = BleState::Connected;
        publish_event(ConnectionStatusChangeEvent(connection));
        let mut running = Box::pin(processor.polling_loop());
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        assert_eq!(running.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(running.as_mut().poll(&mut cx), Poll::Pending);
        driver.advance(embassy_time::Duration::from_micros(16_000));
        assert_eq!(running.as_mut().poll(&mut cx), Poll::Pending);
        drop(running);
        let mut expected = ScreenState::new(Role::Peripheral);
        expected.set_battery(Some(42));
        expected.set_central_connected(true);
        let mut fb = FrameBuffer::new();
        expected.render(&mut fb, 0).unwrap();
        assert_eq!(t.borrow().writes.last().unwrap(), fb.frame(true));
    }

    #[test]
    fn initialized_nonzero_default_layer_seeds_first_image_without_any_layer_event() {
        use crate::runtime::{Central, ScreenProcessor};
        use rmk::{
            config::{BehaviorConfig, PositionalConfig},
            event::{LayerChangeEvent, SubscribableEvent},
            keymap::KeyMap,
            processor::PollingProcessor,
            types::action::KeyAction,
            KeymapData,
        };
        use std::{
            future::Future,
            task::{Context, Poll},
        };
        let _lock = EVENT_LOCK.lock().unwrap();
        let driver = embassy_time::MockDriver::get();
        driver.reset();
        let (p, t) = panel();
        t.borrow_mut().mock_clock = true;
        let mut processor = ScreenProcessor::<_, _, _, _, Central>::new(p, || false);
        let mut layer_probe = LayerChangeEvent::subscriber();
        // Model the behavior.default_layer value restored by RMK storage before
        // KeyMap::build. Real public keymap construction must not need an event.
        let mut behavior = BehaviorConfig {
            default_layer: 2,
            ..Default::default()
        };
        let mut data = KeymapData::new([[[KeyAction::No; 1]; 1]; 4]);
        let positional = PositionalConfig::default();
        let keymap =
            futures::executor::block_on(KeyMap::new(&mut data, &mut behavior, &positional));
        assert_eq!(keymap.active_layer(), 2);
        assert!(layer_probe.try_next_message_pure().is_none());
        processor.seed_from_keymap(&keymap);
        let mut running = Box::pin(processor.polling_loop());
        let mut cx = Context::from_waker(futures::task::noop_waker_ref());
        assert_eq!(running.as_mut().poll(&mut cx), Poll::Pending);
        assert_eq!(running.as_mut().poll(&mut cx), Poll::Pending);
        driver.advance(embassy_time::Duration::from_micros(16_000));
        assert_eq!(running.as_mut().poll(&mut cx), Poll::Pending);
        assert!(layer_probe.try_next_message_pure().is_none());
        let mut expected = ScreenState::new(Role::Central);
        expected.set_layer(2);
        let mut fb = FrameBuffer::new();
        expected.render(&mut fb, 0).unwrap();
        assert_eq!(t.borrow().writes.last().unwrap(), fb.frame(true));
        // Existing early subscriptions still apply subsequent runtime changes.
        rmk::event::publish_event(LayerChangeEvent(1));
        driver.advance(embassy_time::Duration::from_micros(112_000));
        assert_eq!(running.as_mut().poll(&mut cx), Poll::Pending);
        drop(running);
        expected.set_layer(1);
        expected.render(&mut fb, 0).unwrap();
        assert_eq!(t.borrow().writes.last().unwrap(), fb.frame(false));
    }

    fn bolt_pixels(state: &ScreenState) -> Vec<(i32, i32)> {
        image(state)
            .into_iter()
            .filter(|(x, y)| (50..=59).contains(x) && (0..=11).contains(y))
            .collect()
    }

    #[test]
    fn usb_power_bolt_appears_and_erases_without_changing_unknown_or_full_battery() {
        for role in [Role::Central, Role::Peripheral] {
            for battery in [None, Some(100)] {
                let mut state = ScreenState::new(role);
                state.set_battery(battery);
                let original = image(&state);
                let battery_text = state.battery_text();
                assert!(bolt_pixels(&state).is_empty());
                assert!(!state.set_usb_powered(false));
                assert!(state.set_usb_powered(true));
                assert!(!state.set_usb_powered(true));
                let bolt = bolt_pixels(&state);
                assert!(
                    bolt.len() > 15,
                    "white bolt must contain real visible pixels"
                );
                // Extreme tips, center and surrounding black notches distinguish
                // a lightning shape from a filled rectangle or an empty glyph.
                for point in [(57, 0), (50, 6), (51, 11), (54, 5)] {
                    assert!(bolt.contains(&point), "missing bolt pixel {point:?}");
                }
                for point in [(50, 0), (59, 11)] {
                    assert!(!bolt.contains(&point));
                }
                let without_bolt: Vec<_> = image(&state)
                    .into_iter()
                    .filter(|p| !bolt.contains(p))
                    .collect();
                assert_eq!(without_bolt, original);
                assert_eq!(state.battery_text(), battery_text);
                let mut fb = FrameBuffer::new();
                state.render(&mut fb, 0).unwrap();
                assert!(state.set_usb_powered(false));
                state.render(&mut fb, 0).unwrap();
                let mut expected = FrameBuffer::new();
                let mut no_usb = ScreenState::new(role);
                no_usb.set_battery(battery);
                no_usb.render(&mut expected, 0).unwrap();
                assert_eq!(fb.frame(false), expected.frame(false));
                assert_eq!(image(&state), original);
            }
        }
    }

    #[test]
    fn each_processor_samples_its_own_usb_power_without_enumeration_or_remote_status() {
        use crate::runtime::{Central, Peripheral, ScreenEvent, ScreenProcessor};
        use rmk::{
            event::{ConnectionStatus, ConnectionStatusChangeEvent},
            processor::{PollingProcessor, Processor},
            types::{
                battery::{BatteryStatus, ChargeState},
                connection::UsbState,
            },
        };
        use std::sync::atomic::Ordering;
        let _lock = EVENT_LOCK.lock().unwrap();
        let driver = embassy_time::MockDriver::get();
        driver.reset();
        LEFT_USB.store(false, Ordering::Relaxed);
        RIGHT_USB.store(false, Ordering::Relaxed);
        let (p, left) = panel();
        let (p2, right) = panel();
        let mut central = ScreenProcessor::<_, _, _, _, Central>::new(p, left_usb);
        let mut peripheral = ScreenProcessor::<_, _, _, _, Peripheral>::new(p2, right_usb);
        futures::executor::block_on(central.process(ScreenEvent::Battery(
            rmk::event::BatteryStatusEvent(BatteryStatus::Available {
                charge_state: ChargeState::Unknown,
                level: Some(100),
            }),
        )));
        futures::executor::block_on(async {
            central.update().await;
            peripheral.update().await;
        });
        driver.advance(embassy_time::Duration::from_micros(16_000));
        futures::executor::block_on(async {
            central.update().await;
            peripheral.update().await;
        });
        // Wall power on left: no USB/connection event or enumeration is produced.
        LEFT_USB.store(true, Ordering::Relaxed);
        driver.advance(embassy_time::Duration::from_micros(112_000));
        futures::executor::block_on(async {
            central.update().await;
            peripheral.update().await;
        });
        let mut expected = ScreenState::new(Role::Central);
        expected.set_battery(Some(100));
        expected.set_usb_powered(true);
        let mut fb = FrameBuffer::new();
        expected.render(&mut fb, 0).unwrap();
        assert_eq!(left.borrow().writes.last().unwrap(), fb.frame(false));
        assert_eq!(
            right.borrow().writes.last().unwrap().len(),
            2,
            "remote USB must not redraw the right screen"
        );
        // Reverse local USB sources. A remote/enumerated USB snapshot must not
        // override either physical source, including the peripheral's bolt.
        LEFT_USB.store(false, Ordering::Relaxed);
        RIGHT_USB.store(true, Ordering::Relaxed);
        let mut remote = ConnectionStatus::new();
        remote.usb = UsbState::Disabled;
        rmk::event::publish_event(ConnectionStatusChangeEvent(remote));
        driver.advance(embassy_time::Duration::from_micros(112_000));
        futures::executor::block_on(async {
            central.update().await;
            peripheral.update().await;
        });
        expected.set_usb_powered(false);
        expected.render(&mut fb, 0).unwrap();
        assert_eq!(left.borrow().writes.last().unwrap(), fb.frame(true));
        let mut expected_right = ScreenState::new(Role::Peripheral);
        expected_right.set_usb_powered(true);
        expected_right.render(&mut fb, 0).unwrap();
        assert_eq!(right.borrow().writes.last().unwrap(), fb.frame(true));
        assert_eq!(expected_right.battery_text().as_str(), "--%");
        RIGHT_USB.store(false, Ordering::Relaxed);
        remote.usb = UsbState::Configured;
        rmk::event::publish_event(ConnectionStatusChangeEvent(remote));
        driver.advance(embassy_time::Duration::from_micros(112_000));
        futures::executor::block_on(peripheral.update());
        expected_right.set_usb_powered(false);
        expected_right.render(&mut fb, 0).unwrap();
        assert_eq!(right.borrow().writes.last().unwrap(), fb.frame(false));
    }
}
