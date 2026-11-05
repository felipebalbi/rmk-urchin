use crate::{
    nice_view::FrameBuffer,
    screen::{HostLink, Role, Screen, ScreenState},
    tests::{Fault, panel},
};
use core::convert::Infallible;
use embedded_graphics::{pixelcolor::BinaryColor, prelude::*};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Canvas([u8; 1360]);

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
        for Pixel(p, color) in pixels {
            assert!((0..68).contains(&p.x) && (0..160).contains(&p.y));
            let n = p.y as usize * 68 + p.x as usize;
            let bit = 0x80 >> (n % 8);
            if color == BinaryColor::On {
                self.0[n / 8] |= bit;
            } else {
                self.0[n / 8] &= !bit;
            }
        }
        Ok(())
    }
}

fn image(state: &ScreenState) -> Canvas {
    let mut image = Canvas([0; 1360]);
    state.render(&mut image, 0).unwrap();
    image
}

#[test]
fn approved_dark_layout_matches_the_independent_pixel_references() {
    let cases = &[
        (
            Role::Central,
            HostLink::Connected,
            false,
            Some(65),
            include_bytes!("../fixtures/status-ui/central-connected.raw"),
        ),
        (
            Role::Central,
            HostLink::Advertising,
            false,
            Some(65),
            include_bytes!("../fixtures/status-ui/central-advertising.raw"),
        ),
        (
            Role::Central,
            HostLink::Inactive,
            false,
            Some(65),
            include_bytes!("../fixtures/status-ui/central-disconnected.raw"),
        ),
        (
            Role::Peripheral,
            HostLink::Inactive,
            true,
            Some(65),
            include_bytes!("../fixtures/status-ui/peripheral-connected.raw"),
        ),
        (
            Role::Peripheral,
            HostLink::Inactive,
            false,
            Some(65),
            include_bytes!("../fixtures/status-ui/peripheral-disconnected.raw"),
        ),
        (
            Role::Central,
            HostLink::Connected,
            false,
            None,
            include_bytes!("../fixtures/status-ui/central-unknown-battery.raw"),
        ),
        (
            Role::Peripheral,
            HostLink::Inactive,
            true,
            None,
            include_bytes!("../fixtures/status-ui/peripheral-unknown-battery.raw"),
        ),
    ];
    for &(role, host, linked, battery, expected) in cases {
        let mut state = ScreenState::new(role);
        state.set_battery(battery);
        state.set_host(0, host);
        state.set_central_connected(linked);
        let actual = image(&state);
        for n in 0..68 * 160 {
            let bit = 0x80 >> (n % 8);
            assert_eq!(
                actual.0[n / 8] & bit,
                expected[n / 8] & bit,
                "{role:?}, {host:?}, {battery:?}, pixel ({}, {})",
                n % 68,
                n / 68
            );
        }
    }
}

// Reconstruct the panel, not just the packet chosen by the implementation.
#[derive(Clone)]
pub(crate) struct PanelImage([[u8; 20]; 68]);
impl PanelImage {
    pub(crate) fn new() -> Self {
        Self([[0xff; 20]; 68])
    }

    pub(crate) fn apply(&mut self, packet: &[u8]) {
        assert_eq!(packet.last(), Some(&0));
        if packet[0] & 4 != 0 {
            assert_eq!(packet.len(), 2);
            self.0.fill([0xff; 20]);
        } else if packet[0] & 1 != 0 {
            assert!(packet.len() >= 24);
            assert_eq!((packet.len() - 2) % 22, 0);
            let mut last = 0;
            for row in packet[1..packet.len() - 1].as_chunks::<22>().0 {
                assert!(row[0] > last && row[0] <= 68);
                assert_eq!(row[21], 0);
                self.0[row[0] as usize - 1].copy_from_slice(&row[1..21]);
                last = row[0];
            }
        } else {
            assert_eq!(packet.len(), 2);
        }
    }

    pub(crate) fn assert_state(&self, state: &ScreenState) {
        let mut expected = FrameBuffer::new();
        state.render(&mut expected, 0).unwrap();
        let wire = expected.frame(false);
        for row in 0..68 {
            assert_eq!(
                self.0[row],
                wire[2 + row * 22..22 + row * 22],
                "physical row {row}"
            );
        }
    }
}

#[test]
fn field_updates_send_partial_rows_and_erase_previous_values() {
    let (panel, trace) = panel();
    let mut screen = Screen::new(panel, Role::Central);
    screen.state_mut().set_battery(Some(65));
    screen.state_mut().set_host(0, HostLink::Connected);
    futures::executor::block_on(async {
        screen.tick(0).await.unwrap();
        screen.tick(16_000).await.unwrap();
    });
    let mut memory = PanelImage::new();
    for packet in &trace.borrow().writes {
        memory.apply(packet);
    }
    for n in 0..10 {
        match n {
            0 => {
                screen.state_mut().set_layer(2);
            }
            1 => {
                screen.state_mut().set_layer(3);
            }
            2 => {
                screen.state_mut().set_host(5, HostLink::Connected);
            }
            3 => {
                screen.state_mut().set_battery(Some(100));
            }
            4 => {
                screen.state_mut().set_battery(None);
            }
            5 => {
                screen.state_mut().set_usb_powered(true);
            }
            6 => {
                screen.state_mut().set_usb_powered(false);
            }
            7 => {
                screen.state_mut().set_host(5, HostLink::Advertising);
            }
            8 => {
                screen.state_mut().set_host(5, HostLink::Inactive);
            }
            _ => {
                screen.state_mut().set_layer(0);
            }
        }
        futures::executor::block_on(screen.tick(128_000 + n * 112_000)).unwrap();
        let packet = trace.borrow().writes.last().unwrap().clone();
        assert!(
            packet.len() > 2 && packet.len() < 1498,
            "field {n}: {} bytes",
            packet.len()
        );
        if n <= 1 || n == 9 {
            assert!(
                packet[1..packet.len() - 1]
                    .as_chunks::<22>()
                    .0
                    .iter()
                    .all(|row| (7..=62).contains(&row[0]))
            );
        }
        memory.apply(&packet);
        memory.assert_state(screen.state_mut());
    }
}

#[test]
fn advertising_is_static_and_does_not_schedule_extra_image_updates() {
    let (panel, trace) = panel();
    let mut screen = Screen::new(panel, Role::Central);
    screen.state_mut().set_host(0, HostLink::Advertising);
    futures::executor::block_on(async {
        for now in (0..=1_024_000).step_by(16_000) {
            screen.tick(now).await.unwrap();
        }
    });
    assert_eq!(
        trace
            .borrow()
            .writes
            .iter()
            .filter(|w| w[0] & 1 != 0)
            .count(),
        1
    );
    for (n, packet) in trace.borrow().writes.iter().enumerate() {
        assert_eq!(packet[0] & 2, if n % 2 == 0 { 0 } else { 2 });
    }
}

#[test]
fn failed_partial_update_resynchronizes_a_complete_image_on_the_next_tick() {
    let (panel, trace) = panel();
    let mut screen = Screen::new(panel, Role::Central);
    futures::executor::block_on(async {
        screen.tick(0).await.unwrap();
        screen.tick(16_000).await.unwrap();
    });
    screen.state_mut().set_layer(1);
    trace.borrow_mut().fail_after_bytes = Some(27);
    assert_eq!(
        futures::executor::block_on(screen.tick(128_000)),
        Err(Fault::Bus)
    );
    assert!(!trace.borrow().cs);
    trace.borrow_mut().fail_after_bytes = None;
    futures::executor::block_on(screen.tick(144_000)).unwrap();
    assert_eq!(trace.borrow().writes.last().unwrap().len(), 1498);
    let mut memory = PanelImage::new();
    memory.apply(trace.borrow().writes.last().unwrap());
    memory.assert_state(screen.state_mut());
}

#[test]
fn cancelled_partial_update_resynchronizes_a_complete_image_on_the_next_tick() {
    use std::{
        future::Future,
        task::{Context, Poll},
    };
    let (panel, trace) = panel();
    let mut screen = Screen::new(panel, Role::Central);
    futures::executor::block_on(async {
        screen.tick(0).await.unwrap();
        screen.tick(16_000).await.unwrap();
    });
    screen.state_mut().set_layer(1);
    trace.borrow_mut().pending = true;
    let mut transfer = Box::pin(screen.tick(128_000));
    assert_eq!(
        transfer
            .as_mut()
            .poll(&mut Context::from_waker(futures::task::noop_waker_ref())),
        Poll::Pending
    );
    assert!(trace.borrow().cs);
    drop(transfer);
    assert!(!trace.borrow().cs);
    trace.borrow_mut().pending = false;
    futures::executor::block_on(screen.tick(144_000)).unwrap();
    assert_eq!(trace.borrow().writes.last().unwrap().len(), 1498);
}

#[test]
fn every_layer_and_profile_value_stays_inside_the_screen() {
    let mut state = ScreenState::new(Role::Central);
    for layer in [0, 1, 2, 3, 4, 5, 6, 255] {
        state.set_layer(layer);
        for profile in [0, 5, 255] {
            state.set_host(profile, HostLink::Connected);
            let _ = image(&state);
        }
    }
}

#[test]
fn compact_battery_reports_each_percentage_and_changes_no_other_field() {
    for role in [Role::Central, Role::Peripheral] {
        let mut state = ScreenState::new(role);
        state.set_battery(Some(0));
        let baseline = image(&state);
        let mut seen = Vec::new();
        for level in 0..=100 {
            state.set_battery(Some(level));
            let actual = image(&state);
            assert!(
                !seen.contains(&actual),
                "{role:?}: percentage {level} must be distinguishable"
            );
            for n in 0..68 * 160 {
                let (x, y) = (n % 68, n / 68);
                if !(6..52).contains(&x) || !(23..33).contains(&y) {
                    let bit = 0x80 >> (n % 8);
                    assert_eq!(actual.0[n / 8] & bit, baseline.0[n / 8] & bit);
                }
            }
            seen.push(actual);
        }
    }
}

#[test]
fn all_profiles_and_layer_labels_remain_distinguishable() {
    let mut state = ScreenState::new(Role::Central);
    state.set_host(0, HostLink::Connected);
    let mut profiles = Vec::new();
    for profile in 0..6 {
        state.set_host(profile, HostLink::Connected);
        let actual = image(&state);
        assert!(!profiles.contains(&actual), "profile {profile}");
        profiles.push(actual);
    }
    let mut layers = Vec::new();
    for layer in 0..7 {
        state.set_layer(layer);
        let actual = image(&state);
        assert!(!layers.contains(&actual), "layer {layer}");
        layers.push(actual);
    }
}
