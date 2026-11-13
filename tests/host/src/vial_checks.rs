use serde_json::Value;
use std::collections::BTreeSet;

// Vial must expose every physical switch once, not the six unused thumb-row
// matrix slots. Removing a key or assigning it to the wrong slot breaks this
// contract even if the JSON still parses and the firmware still builds.
#[test]
fn vial_exposes_every_urchin_switch_exactly_once() {
    let definition: Value = serde_json::from_str(include_str!("../../../vial.json")).unwrap();
    let rows = definition["matrix"]["rows"].as_u64().unwrap();
    let cols = definition["matrix"]["cols"].as_u64().unwrap();
    assert_eq!((rows, cols), (4, 10));

    let mut switches = BTreeSet::new();
    for row in definition["layouts"]["keymap"].as_array().unwrap() {
        for item in row.as_array().unwrap() {
            // KLE property objects control geometry; strings identify switches.
            if item.is_object() {
                continue;
            }
            let label = item.as_str().expect("key must have a matrix label");
            let (r, c) = label.split_once(',').expect("label must be row,col");
            let position = (r.parse::<u64>().unwrap(), c.parse::<u64>().unwrap());
            assert!(
                position.0 < rows && position.1 < cols,
                "out of bounds: {label}"
            );
            assert!(switches.insert(position), "duplicate switch: {label}");
        }
    }

    // Three ten-key finger rows plus Enter/Nav, Tab/Mouse, Backspace/Sym, Space/Num.
    let expected: BTreeSet<_> = (0..3)
        .flat_map(|row| (0..10).map(move |col| (row, col)))
        .chain([(3, 3), (3, 4), (3, 5), (3, 6)])
        .collect();
    assert_eq!(switches, expected, "Vial layout must cover all 34 switches");
}

#[test]
fn vial_user_slots_match_the_four_profile_firmware_actions() {
    let definition: Value = serde_json::from_str(include_str!("../../../vial.json")).unwrap();
    let names: Vec<_> = definition["customKeycodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|code| code["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "BT0", "BT1", "BT2", "BT3", "NEXT_BT", "PREV_BT", "CLR_BT", "SWITCH"
        ]
    );
}

#[test]
fn vial_unlock_challenge_uses_enter_space_on_outer_physical_thumbs() {
    let config = rmk_config::KeyboardTomlConfig::new_from_toml_path(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../keyboard.toml"
    ));
    let host = config.host();
    assert!(!host.insecure);
    assert_eq!(host.unlock_keys, [[3, 3], [3, 6]]);
}
