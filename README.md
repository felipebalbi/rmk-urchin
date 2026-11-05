# Urchin firmware

Felipe Balbi's RMK-based firmware for the Urchin split keyboard, using
nice!nano controllers, a BLE split link, USB/BLE host connections, and a
nice!view status screen on each half.

`keyboard.toml` defines the matrix, four layers, and key behavior.
`src/central.rs` and `src/peripheral.rs` are the two firmware entry points;
the shared display implementation lives in `src/screen.rs`, `src/screen/`,
`src/nice_view.rs`, and `src/display_spi.rs`.

## Keymap

The base layer is QWERTY with home-row Shift/Alt/GUI/Ctrl modifiers.
Left (1) provides numbers and shortcuts, Right (2) symbols and navigation,
and Tri (3) function keys, Bluetooth controls, reboot, and bootloader access.
Holding both layer thumbs activates Tri.

| Left layer thumb | Left inner thumb | Right inner thumb | Right layer thumb |
|---|---|---|---|
| Tab / hold Left | Enter | Space | Backspace / hold Right |

Thumb combos use a 50 ms window:

- Both left thumbs: Escape.
- Both right thumbs: Delete (also works when Right/Tri replaces Space with RCtrl).
- Inner thumbs: Caps Word (Enter + Space, or Enter + RCtrl on Right/Tri).

Home-row modifiers use buffered tap/hold, 200 ms permissive hold, and
same-hand tap filtering. Thumb layer-taps use 200 ms Normal mode without
hand filtering. Flow-tap and quick-tap repeat are disabled.

## Build and flash

Run from this repository's root:

```sh
cargo install cargo-make             # once
cargo make test
cargo make uf2 --release
```

Put each controller into its UF2 bootloader, then copy
`rmk-urchin-central.uf2` to the central half and
`rmk-urchin-peripheral.uf2` to the peripheral. Flash matching builds to both.

Firmware and host dependencies are pinned in their respective Cargo manifests.
Changes to the RMK revision or Cargo features can clear stored settings and
bonds, requiring host and split re-pairing. Stored/Vial keymaps can override
compiled defaults; back up custom mappings before resetting them.

## Vial

Connect the left (central) half over USB, keep the right half powered, and open
[Vial](https://get.vial.today/download/) or [Vial Web](https://vial.rocks/).
Select `rmk-urchin` to edit any of the four layers. Changes are stored on the
keyboard automatically; use **File → Save current layout** to back them up.

`vial.json` describes all 34 physical keys, including the four thumbs. It is
embedded in the firmware by `build.rs`, so changes to the visual layout require
rebuilding and flashing. Use **File → Load dummy JSON** to preview `vial.json`
without flashing; dummy mode does not edit the connected keyboard.

## Displays and power

Each screen uses a compact dark layout with a battery icon and numeric
percentage, or `?` and `--%` when unknown. The bolt indicates local USB power,
not charging. The central shows the prominent uppercase layer and a compact
zero-based profile (`P0`, `P1`, etc.). Its static host icon distinguishes connected
(plain Bluetooth symbol), advertising (small squares and short arcs on both sides),
and disconnected (an X). The peripheral shows `OK` or `WAIT` for its split link.

![Dark screen previews: central connected, advertising and disconnected; peripheral connected and waiting](assets/screen-states.svg)

Pixel-accurate previews generated from the firmware renderer, enlarged for
readability. Battery values are examples, not measurements.

![Central screen previews for the Base, Left, Right and Tri layers](assets/screen-layers.svg)

Status fields have fixed bounds. Ordinary changes update only affected physical
rows; startup, wake, and recovery still redraw the full screen. Unchanged content
uses hold commands at the same 16 ms panel cadence, without advertising animation.

After 30 seconds idle, the split link uses subrating and the screens clear
and switch off their external supply. On wake, screens reinitialize and
redraw current status. This is not MCU System OFF; peripheral wake can have
additional split-link latency.

USB CDC logging is disabled. To enable it temporarily, add `usb_log` to the
RMK features in `Cargo.toml` and rebuild both halves. The application caps
logs at Debug; changing features can also reset pairing data.

## Tests

Without cargo-make:

```sh
cargo test --manifest-path tests/host/Cargo.toml \
  --target "$(rustc -vV | sed -n 's/^host: //p')" --locked
```

Host checks compile the shared firmware display modules and cover rendering,
SPI recovery/cancellation, screen sleep, 5 ms debounce, and representative
tap/hold typing. Typing regressions include capital I after Space without a
stray A and early Right-thumb Quote without a base-layer M.

On hardware, check fast rolls, deliberate modifier/layer holds, all thumb
combos, and wake after idle on each half. Native tests do not validate
physical timing or radio behavior.
