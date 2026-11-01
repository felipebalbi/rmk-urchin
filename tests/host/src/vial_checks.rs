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

    // Three ten-key finger rows plus Tab/Left, Enter, Space, Backspace/Right.
    let expected: BTreeSet<_> = (0..3)
        .flat_map(|row| (0..10).map(move |col| (row, col)))
        .chain([(3, 3), (3, 4), (3, 5), (3, 6)])
        .collect();
    assert_eq!(switches, expected, "Vial layout must cover all 34 switches");
}
