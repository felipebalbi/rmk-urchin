//! Drive the resolved project keymap and combos, not a representative fixture.
use embassy_time::{Duration, Instant, MockDriver, Timer};
use rmk::{
    config::{BehaviorConfig, Hand, PositionalConfig},
    core_traits::Runnable,
    event::KeyboardEvent,
    hid::Report,
    keyboard::{
        Keyboard,
        combo::{Combo, ComboConfig},
    },
    keymap::{KeyMap, KeymapData},
    types::{
        action::{Action, KeyAction, KeyboardAction},
        keycode::{HidKeyCode as K, KeyCode},
        morse::Morse,
    },
};
use std::{
    future::Future,
    task::{Context, Poll},
};

fn code(s: &str) -> K {
    let name = if s.len() == 1 && s.as_bytes()[0].is_ascii_digit() {
        format!("Kc{s}")
    } else {
        s.to_owned()
    };
    serde_json::from_value(serde_json::json!(name)).unwrap_or_else(|_| panic!("unknown key {s}"))
}

// Small test adapter for the action forms this board uses. Unknown forms fail
// loudly; firmware builds independently exercise RMK's actual code generator.
fn binding(s: &str, names: &[String]) -> KeyAction {
    let s = s.trim();
    if s == "_" {
        return KeyAction::Transparent;
    }
    if s == "No" {
        return KeyAction::No;
    }
    if let Some((kind, args)) = s.split_once('(') {
        let args: Vec<_> = args
            .trim_end_matches(')')
            .split(',')
            .map(str::trim)
            .collect();
        return match kind {
            "LT" => KeyAction::TapHold(
                Action::Key(KeyCode::Hid(code(args[1]))),
                Action::LayerOn(args[0].parse().unwrap()),
                args.get(2).map_or(255, |name| {
                    names.iter().position(|n| n == name).unwrap() as u8
                }),
            ),
            "MT" => KeyAction::TapHold(
                Action::Key(KeyCode::Hid(code(args[0]))),
                Action::Modifier(code(args[1]).to_hid_modifiers()),
                255,
            ),
            "WM" => KeyAction::Single(Action::KeyWithModifier(
                code(args[0]),
                args[1].split('|').fold(Default::default(), |mods, m| {
                    mods | code(m.trim()).to_hid_modifiers()
                }),
            )),
            "TD" => KeyAction::Morse(args[0].parse().unwrap()),
            _ => panic!("unsupported action {s}"),
        };
    }
    let action = match s {
        "CapsWordToggle" => Action::KeyboardControl(KeyboardAction::CapsWordToggle),
        "Bootloader" => Action::KeyboardControl(KeyboardAction::Bootloader),
        "Reboot" => Action::KeyboardControl(KeyboardAction::Reboot),
        s if s.starts_with("User") => Action::User(s[4..].parse().unwrap()),
        _ => Action::Key(KeyCode::Hid(code(s))),
    };
    KeyAction::Single(action)
}

#[derive(Default, Debug)]
struct Trace {
    held: [u8; 6],
    keys: Vec<(K, u8)>,
    modifiers: Vec<u8>,
    media: Vec<u16>,
    buttons: Vec<u8>,
    layers: Vec<u8>,
}

fn drive<F: Future>(future: F, trace: &mut Trace) -> F::Output {
    let mut future = std::pin::pin!(future);
    let mut cx = Context::from_waker(futures::task::noop_waker_ref());
    for _ in 0..50_000 {
        let result = future.as_mut().poll(&mut cx);
        while let Ok(report) = rmk::channel::USB_REPORT_CHANNEL.try_receive() {
            match report {
                Report::KeyboardReport(r) => {
                    trace.modifiers.push(r.modifier);
                    for key in r.keycodes {
                        if key != 0 && !trace.held.contains(&key) {
                            trace.keys.push((K::from(key), r.modifier));
                        }
                    }
                    trace.held = r.keycodes;
                }
                Report::MediaKeyboardReport(r) => trace.media.push(r.usage_id),
                Report::MouseReport(r) => trace.buttons.push(r.buttons),
                other => panic!("unexpected report {other:?}"),
            }
        }
        if let Poll::Ready(value) = result {
            return value;
        }
        MockDriver::get().advance(Duration::from_micros(100));
    }
    panic!("keyboard stalled for five seconds of virtual time");
}

// Absolute times allow checking the combo window and release order precisely.
fn sequence(events: &[(u64, u8, u8, bool)]) -> Trace {
    let _lock = crate::tests::EVENT_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    MockDriver::get().reset();
    rmk::state::set_usb_state(rmk::types::connection::UsbState::Configured);
    rmk::channel::USB_REPORT_CHANNEL.clear();
    let config = rmk_config::KeyboardTomlConfig::new_from_toml_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../keyboard.toml"
    ));
    let resolved = config.keymap().unwrap();
    let behavior_config = config.behavior().unwrap();
    let morse = behavior_config.morse.unwrap();
    let mut names: Vec<_> = morse.profiles.keys().cloned().collect();
    names.sort();
    let mut behavior = BehaviorConfig::default();
    behavior.tri_layer = behavior_config.tri_layer;
    behavior.morse.enable_flow_tap = morse.enable_flow_tap;
    behavior.morse.prior_idle_time = Duration::from_millis(morse.prior_idle_time_ms);
    behavior.morse.default_profile = crate::typing_checks::profile(&morse.default_profile);
    for name in &names {
        behavior
            .morse
            .profiles
            .push(crate::typing_checks::profile(&morse.profiles[name]))
            .unwrap();
    }
    let action = |s: Option<&str>| match binding(s.unwrap_or("No"), &names) {
        KeyAction::Single(a) => a,
        KeyAction::No => Action::No,
        other => panic!("not a simple action: {other:?}"),
    };
    for m in &morse.morses {
        let p = m
            .profile
            .as_ref()
            .map(|n| crate::typing_checks::profile(&morse.profiles[n]))
            .unwrap_or_default();
        behavior
            .morse
            .morses
            .push(Morse::new_from_vial(
                action(m.tap.as_deref()),
                action(m.hold.as_deref()),
                action(m.hold_after_tap.as_deref()),
                action(m.double_tap.as_deref()),
                p,
            ))
            .unwrap();
    }
    if let Some(combos) = behavior_config.combos {
        behavior.combo.timeout = Duration::from_millis(combos.timeout_ms.unwrap_or(50));
        assert!(combos.combos.len() <= behavior.combo.combos.len());
        for (i, c) in combos.combos.iter().enumerate() {
            behavior.combo.combos[i] = Some(Combo::new(ComboConfig::new(
                c.actions.iter().map(|a| binding(a, &names)),
                binding(&c.output, &names),
                c.layer,
            )));
        }
    }
    let mut map = [[[KeyAction::No; 10]; 4]; 7];
    for (l, layer) in resolved.keymap.iter().enumerate() {
        for (r, row) in layer.iter().enumerate() {
            for (c, a) in row.iter().enumerate() {
                map[l][r][c] = binding(a, &names);
            }
        }
    }
    let mut data = KeymapData::new(map);
    let positions = PositionalConfig::new(core::array::from_fn(|r| {
        core::array::from_fn(|c| match resolved.key_info[r][c].hand {
            'L' => Hand::Left,
            'R' => Hand::Right,
            _ => Hand::Unknown,
        })
    }));
    let mut trace = Trace::default();
    let keymap = drive(
        KeyMap::new(&mut data, &mut behavior, &positions),
        &mut trace,
    );
    let mut keyboard = Keyboard::new(&keymap);
    MockDriver::get().advance(Duration::from_secs(1));
    let started = Instant::now();
    for &(ms, row, col, pressed) in events {
        let deadline = started + Duration::from_millis(ms);
        assert!(Instant::now() <= deadline, "input timeline passed {ms}ms");
        drive(
            embassy_futures::select::select(keyboard.run(), Timer::at(deadline)),
            &mut trace,
        );
        drive(
            keyboard.process_inner(KeyboardEvent::key(row, col, pressed)),
            &mut trace,
        );
        trace.layers.push(keymap.active_layer());
    }
    // Let delayed releases and combo/morse deadlines drain.
    drive(
        embassy_futures::select::select(keyboard.run(), Timer::after(Duration::from_millis(300))),
        &mut trace,
    );
    trace.layers.push(keymap.active_layer());
    trace
}

fn released(trace: &Trace) {
    assert_eq!(trace.held, [0; 6], "{trace:?}");
    assert_eq!(trace.layers.last(), Some(&0), "layer stuck: {trace:?}");
    assert!(
        trace.modifiers.last().is_none_or(|m| *m == 0),
        "modifier stuck: {trace:?}"
    );
}

#[test]
fn tertiary_chords_tap_escape_delete_without_individual_thumbs_in_either_order() {
    for (a, b, tap) in [(3, 4, K::Escape), (5, 6, K::Delete)] {
        for (first, second) in [(a, b), (b, a)] {
            for (up1, up2) in [(a, b), (b, a)] {
                let t = sequence(&[
                    (0, 3, first, true),
                    (20, 3, second, true),
                    (70, 3, up1, false),
                    (90, 3, up2, false),
                ]);
                assert_eq!(t.keys, [(tap, 0)], "{t:?}");
                released(&t);
            }
        }
    }
}

#[test]
fn tertiary_chord_holds_reach_media_and_fun_without_tap_leaks() {
    for (a, b, layer, row, col, key, media) in [
        (3, 4, 3, 1, 7, None, Some(0xea)), // Media: volume down
        (5, 6, 6, 2, 1, Some(K::F1), None),
    ] {
        for (first, second) in [(a, b), (b, a)] {
            for (up1, up2) in [(a, b), (b, a)] {
                let t = sequence(&[
                    (0, 3, first, true),
                    (20, 3, second, true),
                    (250, row, col, true),
                    (280, row, col, false),
                    (320, 3, up1, false),
                    (350, 3, up2, false),
                ]);
                assert!(t.layers.contains(&layer), "{t:?}");
                assert_eq!(
                    t.layers[4], layer,
                    "releasing only one member must retain the layer: {t:?}"
                );
                assert_eq!(
                    t.keys,
                    key.map(|k| (k, 0)).into_iter().collect::<Vec<_>>(),
                    "{t:?}"
                );
                if let Some(usage) = media {
                    assert_eq!(t.media, [usage, 0]);
                }
                released(&t);
            }
        }
    }
}

#[test]
fn four_physical_thumbs_keep_taps_and_reach_primary_secondary_layers() {
    for (col, tap, layer, row, keycol, key, mods) in [
        (3, K::Tab, 2, 0, 9, K::Undo, 0),
        (4, K::Enter, 1, 1, 6, K::Left, 0),
        (5, K::Space, 4, 1, 0, K::Quote, 0),
        (6, K::Backspace, 5, 1, 0, K::Quote, 2),
    ] {
        let t = sequence(&[(0, 3, col, true), (80, 3, col, false)]);
        assert_eq!(t.keys, [(tap, 0)]);
        released(&t);
        let t = sequence(&[
            (0, 3, col, true),
            (230, row, keycol, true),
            (280, row, keycol, false),
            (330, 3, col, false),
        ]);
        assert_eq!(t.keys, [(key, mods)], "{t:?}");
        assert!(t.layers.contains(&layer));
        released(&t);
    }
}

#[test]
fn opposite_hand_tertiary_actions_survive_on_sublayers() {
    // Left chord: Num dot, Sym open parenthesis, Fun Menu.
    for (selector, key, mods, chord) in [
        (5, K::Dot, 0, false),
        (6, K::Kc9, 2, false),
        (5, K::Application, 0, true),
    ] {
        let mut events = vec![(0, 3, selector, true)];
        if chord {
            events.push((20, 3, 6, true));
        }
        events.extend([
            (250, 3, 3, true),
            (270, 3, 4, true),
            (320, 3, 3, false),
            (340, 3, 4, false),
            (400, 3, selector, false),
        ]);
        if chord {
            events.push((420, 3, 6, false));
        }
        let t = sequence(&events);
        assert_eq!(t.keys, [(key, mods)], "{t:?}");
        released(&t);
    }
    // Right chord: Nav Delete, Mouse middle button, Media mute.
    for (selector, key, media, button, chord) in [
        (4, Some(K::Delete), None, None, false),
        (3, None, None, Some(4), false),
        (3, None, Some(0xe2), None, true),
    ] {
        let mut events = vec![(0, 3, selector, true)];
        if chord {
            events.push((20, 3, 4, true));
        }
        events.extend([
            (250, 3, 5, true),
            (270, 3, 6, true),
            (320, 3, 5, false),
            (340, 3, 6, false),
            (400, 3, selector, false),
        ]);
        if chord {
            events.push((420, 3, 4, false));
        }
        let t = sequence(&events);
        assert_eq!(
            t.keys,
            key.map(|k| (k, 0)).into_iter().collect::<Vec<_>>(),
            "{t:?}"
        );
        if let Some(usage) = media {
            assert_eq!(t.media, [usage, 0]);
        }
        if let Some(mask) = button {
            assert_eq!(t.buttons, [mask, 0]);
        }
        released(&t);
    }
}

#[test]
fn combo_window_separates_tertiary_chords_from_individual_thumb_rolls() {
    for (a, b, tap_a, tap_b, tertiary) in [
        (3, 4, K::Tab, K::Enter, K::Escape),
        (5, 6, K::Space, K::Backspace, K::Delete),
    ] {
        for (first, second, first_tap, second_tap) in [(a, b, tap_a, tap_b), (b, a, tap_b, tap_a)] {
            let t = sequence(&[
                (0, 3, first, true),
                (49, 3, second, true),
                (90, 3, first, false),
                (110, 3, second, false),
            ]);
            assert_eq!(t.keys, [(tertiary, 0)], "inside combo window: {t:?}");
            released(&t);
            let t = sequence(&[
                (0, 3, first, true),
                (51, 3, second, true),
                (90, 3, first, false),
                (110, 3, second, false),
            ]);
            assert_eq!(
                t.keys,
                [(first_tap, 0), (second_tap, 0)],
                "outside combo window: {t:?}"
            );
            released(&t);
        }
    }
}

#[test]
fn early_keys_wait_for_primary_secondary_and_tertiary_layer_holds() {
    // Key-down/up before the 200ms timeout must not escape onto Base.
    for (first, second, row, col, key, mods) in [
        (4, None, 1, 6, K::Left, 0),
        (5, None, 1, 0, K::Quote, 0),
        (6, None, 1, 0, K::Quote, 2),
        (5, Some(6), 2, 1, K::F1, 0),
    ] {
        let mut events = vec![(0, 3, first, true)];
        if let Some(second) = second {
            events.push((20, 3, second, true));
        }
        events.extend([
            (89, row, col, true),
            (167, row, col, false),
            (300, 3, first, false),
        ]);
        if let Some(second) = second {
            events.push((320, 3, second, false));
        }
        let t = sequence(&events);
        assert_eq!(t.keys, [(key, mods)], "{t:?}");
        released(&t);
    }
}

#[test]
fn caps_word_is_on_nav_h_and_inner_thumb_taps_remain_enter_space() {
    let t = sequence(&[
        (0, 3, 4, true),
        (250, 1, 5, true),
        (280, 1, 5, false),
        (320, 3, 4, false),
        (400, 0, 0, true),
        (430, 0, 0, false),
    ]);
    assert_eq!(t.keys, [(K::Q, 2)], "{t:?}");
    released(&t);
    let t = sequence(&[
        (0, 3, 4, true),
        (20, 3, 5, true),
        (70, 3, 4, false),
        (90, 3, 5, false),
    ]);
    assert_eq!(t.keys, [(K::Enter, 0), (K::Space, 0)], "{t:?}");
    released(&t);
}

#[test]
fn clipboard_uses_ctrl_y_redo_and_portable_editing_shortcuts() {
    for selector in [3, 4] {
        for (col, key, mods) in [
            (5, K::Y, 1),
            (6, K::Insert, 2),
            (7, K::Insert, 1),
            (8, K::Delete, 2),
            (9, K::Undo, 0),
        ] {
            let t = sequence(&[
                (0, 3, selector, true),
                (250, 0, col, true),
                (300, 0, col, false),
                (350, 3, selector, false),
            ]);
            assert_eq!(t.keys, [(key, mods)], "{t:?}");
            released(&t);
        }
    }
}

#[test]
fn media_user_actions_match_four_profile_firmware_and_transport_toggle() {
    let config = rmk_config::KeyboardTomlConfig::new_from_toml_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../keyboard.toml"
    ));
    assert_eq!(config.build_constants(&[]).unwrap().ble_profiles_num, 4);
    let keymap = config.keymap().unwrap();
    for (col, id) in [(5, 7), (6, 0), (7, 1), (8, 2), (9, 3)] {
        assert_eq!(
            binding(&keymap.keymap[3][2][col], &[]),
            KeyAction::Single(Action::User(id))
        );
    }
    // Miryoku's unsupported RGB/external-power positions must not clear bonds.
    for (row, col) in (5..10).map(|col| (0, col)).chain([(1, 5)]) {
        assert_eq!(binding(&keymap.keymap[3][row][col], &[]), KeyAction::No);
    }
}
