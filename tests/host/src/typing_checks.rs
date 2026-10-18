//! Representative tap/hold keys driven through the selected RMK engine.
//! Read the project's actual morse settings with RMK's configuration resolver;
//! only the representative key fixture and clock/host transport are test-specific. This is
//! not the upstream full-keyboard simulator and does not exercise thumb combos.
use embassy_time::{Duration, Instant, MockDriver, Timer};
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
    .with_buffered_tap_hold(p.buffered_tap_hold)
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
    key_downs: Vec<(K, u8)>,
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
                    trace.key_downs.push((K::from(key), report.modifier));
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
    /// Absolute milliseconds since the sequence starts, without Down/Up's 30ms gap.
    At(u64, u8, bool),
}
use Step::{At, Down, Up, Wait};

#[derive(Clone, Copy)]
enum Policy {
    Configured,
    LegacyFlowTap,
}

fn sequence(prior_gap_ms: Option<u64>, steps: &[Step]) -> Trace {
    sequence_for(Policy::Configured, prior_gap_ms, steps)
}

fn sequence_for(policy: Policy, prior_gap_ms: Option<u64>, steps: &[Step]) -> Trace {
    let _lock = crate::tests::EVENT_LOCK.lock().unwrap();
    MockDriver::get().reset();
    rmk::state::set_usb_state(rmk::types::connection::UsbState::Configured);
    rmk::channel::USB_REPORT_CHANNEL.clear();
    let (mut behavior, thumb) = configured_behavior();
    if matches!(policy, Policy::LegacyFlowTap) {
        behavior.morse.enable_flow_tap = true;
        behavior.morse.default_profile = behavior
            .morse
            .default_profile
            .with_enable_flow_tap(Some(true))
            .with_buffered_tap_hold(Some(false));
        for profile in &mut behavior.morse.profiles {
            *profile = profile.with_buffered_tap_hold(Some(false));
        }
    }
    let key = |k| Action::Key(KeyCode::Hid(k));
    // 0=A/Shift, 1=S/Alt, 2=Q, 3=H, 4=Tab/layer 1,
    // 5=Backspace/layer 2, 6=I, 7=Space, 8=M (double-quote on layer 2).
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
        KeyAction::Single(key(K::I)),
        KeyAction::Single(key(K::Space)),
        KeyAction::Single(key(K::M)),
    ]];
    let mut right = [[KeyAction::Transparent; 9]];
    right[0][8] = KeyAction::Single(Action::KeyWithModifier(
        K::Quote,
        ModifierCombination::LSHIFT,
    ));
    let mut data = KeymapData::new([base, [[KeyAction::Transparent; 9]], right]);
    let positions = PositionalConfig::new([[
        Hand::Left,
        Hand::Left,
        Hand::Left,
        Hand::Right,
        Hand::Left,
        Hand::Right,
        Hand::Right,
        Hand::Right,
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
    let started = Instant::now();
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
            At(ms, col, pressed) => {
                let deadline = started + Duration::from_millis(ms);
                assert!(Instant::now() <= deadline, "input timeline ran past {ms}ms");
                drive(
                    embassy_futures::select::select(keyboard.run(), Timer::at(deadline)),
                    &mut trace,
                );
                drive(
                    keyboard.process_inner(KeyboardEvent::key(0, col, pressed)),
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
    let trace = sequence_for(
        Policy::LegacyFlowTap,
        Some(30),
        &[Down(0), Down(3), Up(3), Up(0)],
    );
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
        let trace = sequence_for(
            Policy::LegacyFlowTap,
            Some(30),
            &[Down(thumb), Wait(250), Up(thumb)],
        );
        assert_eq!(trace.presses, [tap]);
        assert!(trace.layers.iter().all(|layer| *layer == 0));
        assert_eq!(trace.held, [0; 6]);
    }
}

#[test]
fn capital_i_after_space_does_not_emit_a_from_the_shift_key() {
    // USB capture at 229899-230267ms: Space, A/Shift 119ms later, I another
    // 97ms later. A/Shift stays down for 246ms but flow-tap sends unshifted AI.
    let trace = sequence(
        None,
        &[
            At(0, 7, true),
            At(76, 7, false),
            At(119, 0, true),
            At(216, 6, true),
            At(269, 6, false),
            At(365, 0, false),
        ],
    );
    assert_eq!(
        trace.presses,
        [K::Space, K::I],
        "holding A/Shift to capitalize I must not insert A"
    );
    assert_eq!(
        trace.key_downs,
        [(K::Space, 0), (K::I, 2)],
        "I must be emitted with left Shift, not as lowercase i"
    );
    assert_eq!(trace.held, [0; 6]);
    assert_eq!(trace.modifiers.last(), Some(&0));
}

#[test]
fn capital_i_after_idle_has_shift_without_inserting_a() {
    let trace = sequence(
        None,
        &[
            At(0, 0, true),
            At(97, 6, true),
            At(150, 6, false),
            At(246, 0, false),
        ],
    );
    assert_eq!(trace.key_downs, [(K::I, 2)]);
    assert_eq!(trace.held, [0; 6]);
    assert_eq!(trace.modifiers.last(), Some(&0));
}

#[test]
fn early_right_thumb_quote_chord_does_not_emit_base_layer_m() {
    // The log's 253725-254010ms thumb/R sequence emits a base key at +89ms,
    // before layer 2 activates at +201ms. This uses that timing with the M/quote
    // position from the user's report; the exact failed quote was not captured.
    let trace = sequence(
        None,
        &[
            At(0, 5, true),
            At(89, 8, true),
            At(167, 8, false),
            At(285, 5, false),
        ],
    );
    assert_eq!(
        trace.presses,
        [K::Quote],
        "a key chorded with the held Right thumb must use its layer, not emit M"
    );
    assert_eq!(
        trace.key_downs,
        [(K::Quote, 2)],
        "double-quote requires Shift+Quote"
    );
    assert_eq!(trace.held, [0; 6]);
    assert_eq!(trace.modifiers.last(), Some(&0));
    assert_eq!(trace.layers.last(), Some(&0));
}

#[test]
fn right_thumb_quote_after_hold_timeout_uses_shifted_quote() {
    let trace = sequence(
        None,
        &[
            At(0, 5, true),
            At(210, 8, true),
            At(280, 8, false),
            At(330, 5, false),
        ],
    );
    assert_eq!(trace.key_downs, [(K::Quote, 2)]);
    assert_eq!(trace.held, [0; 6]);
    assert_eq!(trace.modifiers.last(), Some(&0));
    assert_eq!(trace.layers.last(), Some(&0));
}
