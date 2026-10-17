//! Representative tap/hold keys driven through published RMK's real engine.
//! Read the project's actual morse settings with RMK's configuration resolver;
//! only the six-key fixture and clock/host transport are test-specific. This is
//! not the upstream full-keyboard simulator and does not exercise thumb combos.
use embassy_time::{Duration, MockDriver, Timer};
use rmk::{
    config::{BehaviorConfig, Hand, PositionalConfig},
    core_traits::Runnable,
    event::KeyboardEvent,
    hid::Report,
    keyboard::Keyboard,
    keymap::{KeyMap, KeymapData},
    types::{
        action::{Action, KeyAction},
        keycode::{HidKeyCode as K, KeyCode},
        modifier::ModifierCombination,
        morse::{MorseMode, MorseProfile},
    },
};
use std::{
    future::Future,
    task::{Context, Poll},
};

fn profile(p: &rmk_config::resolved::behavior::MorseProfile) -> MorseProfile {
    let mode = if p.permissive_hold == Some(true) {
        Some(MorseMode::PermissiveHold)
    } else if p.hold_on_other_press == Some(true) {
        Some(MorseMode::HoldOnOtherPress)
    } else if p.normal_mode == Some(true) {
        Some(MorseMode::Normal)
    } else {
        None
    };
    MorseProfile::new(
        p.unilateral_tap,
        mode,
        p.hold_timeout_ms.map(|t| t.try_into().unwrap()),
        p.gap_timeout_ms.map(|t| t.try_into().unwrap()),
    )
    .with_enable_flow_tap(p.enable_flow_tap)
    .with_quick_tap_timeout_ms(p.quick_tap_timeout_ms.map(|t| t.try_into().unwrap()))
}

fn configured_behavior() -> (BehaviorConfig, u8) {
    let config = rmk_config::KeyboardTomlConfig::new_from_toml_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../keyboard.toml"
    ));
    let resolved = config.behavior().unwrap().morse.unwrap();
    let mut behavior = BehaviorConfig::default();
    behavior.morse.enable_flow_tap = resolved.enable_flow_tap;
    behavior.morse.prior_idle_time = Duration::from_millis(resolved.prior_idle_time_ms);
    behavior.morse.default_profile = profile(&resolved.default_profile);
    let mut names: Vec<_> = resolved.profiles.keys().collect();
    names.sort();
    let thumb = names
        .iter()
        .position(|name| name.as_str() == "THUMB")
        .unwrap();
    for name in names {
        behavior
            .morse
            .profiles
            .push(profile(&resolved.profiles[name]))
            .unwrap();
    }
    (behavior, thumb.try_into().unwrap())
}

#[derive(Default)]
struct Trace {
    held: [u8; 6],
    presses: Vec<K>,
    modifiers: Vec<u8>,
    layers: Vec<u8>,
}

// Poll real asynchronous key handling while servicing the host report channel
// and clock. A bounded driver prevents broken timers from hanging cargo test.
fn drive<F: Future>(future: F, trace: &mut Trace) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    for _ in 0..50_000 {
        let result = future.as_mut().poll(&mut cx);
        while let Ok(report) = rmk::channel::USB_REPORT_CHANNEL.try_receive() {
            let Report::KeyboardReport(report) = report else {
                panic!("unexpected non-keyboard HID report");
            };
            trace.modifiers.push(report.modifier);
            for key in report.keycodes {
                if key != 0 && !trace.held.contains(&key) {
                    trace.presses.push(K::from(key));
                }
            }
            trace.held = report.keycodes;
        }
        if let Poll::Ready(value) = result {
            return value;
        }
        MockDriver::get().advance(Duration::from_micros(100));
    }
    panic!("keyboard operation stalled for five seconds of virtual time");
}

#[derive(Clone, Copy)]
enum Step {
    Down(u8),
    Up(u8),
    Wait(u64),
}
use Step::{Down, Up, Wait};

fn sequence(prior_gap_ms: Option<u64>, steps: &[Step]) -> Trace {
    let _lock = crate::tests::EVENT_LOCK.lock().unwrap();
    MockDriver::get().reset();
    rmk::state::set_usb_state(rmk::types::connection::UsbState::Configured);
    rmk::channel::USB_REPORT_CHANNEL.clear();
    let (mut behavior, thumb) = configured_behavior();
    let key = |k| Action::Key(KeyCode::Hid(k));
    // A/Shift, S/Alt, Q, H, Tab/layer 1, Backspace/layer 2.
    // Index 255 selects the global profile (outside the named-profile table).
    let base = [[
        KeyAction::TapHold(
            key(K::A),
            Action::Modifier(ModifierCombination::LSHIFT),
            255,
        ),
        KeyAction::TapHold(key(K::S), Action::Modifier(ModifierCombination::LALT), 255),
        KeyAction::Single(key(K::Q)),
        KeyAction::Single(key(K::H)),
        KeyAction::TapHold(key(K::Tab), Action::LayerOn(1), thumb),
        KeyAction::TapHold(key(K::Backspace), Action::LayerOn(2), thumb),
    ]];
    let mut data = KeymapData::new([
        base,
        [[KeyAction::Transparent; 6]],
        [[KeyAction::Transparent; 6]],
    ]);
    let positions = PositionalConfig::new([[
        Hand::Left,
        Hand::Left,
        Hand::Left,
        Hand::Right,
        Hand::Left,
        Hand::Right,
    ]]);
    let mut trace = Trace::default();
    let keymap = drive(
        KeyMap::new(&mut data, &mut behavior, &positions),
        &mut trace,
    );
    let mut keyboard = Keyboard::new(&keymap);
    // Keyboard::new initializes its last-key timestamp to now. Wait after
    // construction so the first key is genuinely idle, not a startup flow-tap.
    MockDriver::get().advance(Duration::from_secs(1));
    if let Some(gap) = prior_gap_ms {
        drive(
            keyboard.process_inner(KeyboardEvent::key(0, 3, true)),
            &mut trace,
        );
        MockDriver::get().advance(Duration::from_millis(10));
        drive(
            keyboard.process_inner(KeyboardEvent::key(0, 3, false)),
            &mut trace,
        );
        MockDriver::get().advance(Duration::from_millis(gap));
        trace = Trace::default();
    }
    for step in steps {
        match *step {
            Down(col) | Up(col) => {
                MockDriver::get().advance(Duration::from_millis(30));
                drive(
                    keyboard.process_inner(KeyboardEvent::key(0, col, matches!(step, Down(_)))),
                    &mut trace,
                );
            }
            Wait(ms) => {
                // Run RMK's real timeout loop, not just its key-down handler.
                drive(
                    embassy_futures::select::select(
                        keyboard.run(),
                        Timer::after(Duration::from_millis(ms)),
                    ),
                    &mut trace,
                );
            }
        }
        trace.layers.push(keymap.active_layer());
    }
    trace
}

#[test]
fn home_row_roll_during_typing_preserves_asq_order_without_modifiers() {
    let trace = sequence(Some(30), &[Down(0), Down(1), Down(2), Up(0), Up(1), Up(2)]);
    assert_eq!(trace.presses, [K::A, K::S, K::Q]);
    assert!(trace.modifiers.iter().all(|m| *m == 0));
    assert_eq!(trace.held, [0; 6]);
}

#[test]
fn thumb_rolls_during_typing_emit_tab_or_backspace_before_the_next_letter() {
    for (thumb, next, want) in [(4, 2, [K::Tab, K::Q]), (5, 3, [K::Backspace, K::H])] {
        let trace = sequence(Some(30), &[Down(thumb), Down(next), Up(thumb), Up(next)]);
        assert_eq!(trace.presses, want);
        assert!(trace.layers.iter().all(|layer| *layer == 0));
        assert_eq!(trace.held, [0; 6]);
    }
}

#[test]
fn nested_opposite_hand_key_during_typing_does_not_turn_a_into_shift() {
    let trace = sequence(Some(30), &[Down(0), Down(3), Up(3), Up(0)]);
    assert_eq!(trace.presses, [K::A, K::H]);
    assert!(trace.modifiers.iter().all(|m| *m == 0));
    assert_eq!(trace.held, [0; 6]);
}

#[test]
fn deliberate_home_row_hold_still_works_after_idle_or_a_typing_pause() {
    for gap in [None, Some(160)] {
        let trace = sequence(gap, &[Down(0), Wait(210), Up(0)]);
        assert!(trace.presses.is_empty());
        assert!(
            trace.modifiers.contains(&2),
            "intentional Shift must activate"
        );
        assert_eq!(trace.modifiers.last(), Some(&0));
    }
}

#[test]
fn deliberate_thumb_layer_holds_still_work_after_a_typing_pause() {
    for (thumb, layer) in [(4, 1), (5, 2)] {
        let trace = sequence(Some(160), &[Down(thumb), Wait(210), Up(thumb)]);
        assert!(trace.presses.is_empty());
        assert!(trace.layers.contains(&layer));
        assert_eq!(trace.layers.last(), Some(&0));
    }
}

#[test]
fn thumb_started_during_typing_stays_a_tap_even_if_held_past_the_timeout() {
    for (thumb, tap) in [(4, K::Tab), (5, K::Backspace)] {
        let trace = sequence(Some(30), &[Down(thumb), Wait(250), Up(thumb)]);
        assert_eq!(trace.presses, [tap]);
        assert!(trace.layers.iter().all(|layer| *layer == 0));
        assert_eq!(trace.held, [0; 6]);
    }
}
