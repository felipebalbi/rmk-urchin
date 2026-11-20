# Urchin firmware

Felipe Balbi's RMK-based firmware for the Urchin split keyboard, using
nice!nano controllers, a BLE split link, USB/BLE host connections, and a
nice!view status screen on each half.

`keyboard.toml` defines the matrix, seven layers, and key behavior.
`src/central.rs` and `src/peripheral.rs` are the two firmware entry points;
the shared display implementation lives in `src/screen.rs`, `src/screen/`,
`src/nice_view.rs`, and `src/display_spi.rs`.

## Keymap

![Urchin keymap with all seven layers, tap/hold legends, and tertiary thumb chords](assets/keymap.svg)

The diagram shows the compiled defaults in `keyboard.toml`, using schematic
34-key placement and US symbol legends. Regenerate it with
`python3 assets/generate-keymap.py` (Python 3.11 or newer).

This ports the QWERTY layout from [rmk-remake](https://github.com/felipebalbi/rmk-remake),
based on [Miryoku's layer structure](https://github.com/manna-harbour/miryoku/tree/master/docs/reference).
Urchin has **two thumbs per hand**, so a same-hand two-thumb chord takes the
place of Miryoku's **tertiary** (third) thumb. This is unrelated to Miryoku's
alternate alpha layer named Extra, which is not enabled here. There is no Tri layer.

Thumbs match REMAKE's order with its two tertiary switches omitted, from left
to right (outer left, inner left, inner right, outer right):

| Left Enter thumb | Left Tab thumb | Right Backspace thumb | Right Space thumb |
|---|---|---|---|
| Enter / hold Nav | Tab / hold Mouse | Backspace / hold Sym | Space / hold Num |
| `(3,3)` | `(3,4)` | `(3,5)` | `(3,6)` |

| Layer | Hold | Opposite-hand functions |
|---|---|---|
| Nav (1) | Left Enter | Clipboard, arrows, line/page movement, Caps Word |
| Mouse (2) | Left Tab | Pointer, wheel, mouse buttons, clipboard |
| Media (3) | **Both left thumbs** | Playback, volume, Bluetooth and USB/BLE controls |
| Num (4) | Right Space | Numpad and unshifted punctuation |
| Sym (5) | Right Backspace | Shifted symbols in matching Num positions |
| Fun (6) | **Both right thumbs** | Function keys, Print Screen, Scroll Lock, Pause, Menu |

Home-row mods remain Shift on A/semicolon, Alt on S/L, GUI on D/K, and Ctrl on
F/J. Supporting-hand sublayers have dedicated modifiers in these same positions;
opposite-hand keys are single-function. `No` keys are disabled, not transparent.

### Tertiary thumb chords

Press both thumbs **on the same hand within 50 ms**. Tap and release for the
tertiary tap; hold the chord for 200 ms to activate its layer. Release **both**
members to release the chord output/layer. Either press order and either release
order work. A chord replaces, rather than adds to, its individual thumb actions.
Presses outside the combo window remain separate thumb actions.
Pressing another key while the chord is down selects its hold layer immediately,
so fast Media/Fun chords do not leak a Base key before the timeout. The 200 ms
timeout starts when the second thumb completes the chord.

| Active layer | Both left thumbs | Both right thumbs |
|---|---|---|
| Base | **Escape / hold Media** | **Delete / hold Fun** |
| Nav | Disabled | Delete (hold for repeat) |
| Mouse | Disabled | Middle mouse button (hold for drag) |
| Media | Disabled | Mute |
| Num | Dot | Disabled |
| Sym | `(` | Disabled |
| Fun | Menu / Application | Disabled |

Thus every action of REMAKE's missing tertiary thumbs remains available, not
just base Escape and Delete. Left Enter/Tab also become 0/Minus on Num,
close parenthesis/underscore on Sym, and Enter/Tab on Fun. Right Backspace/Space
become Backspace/Space on Nav, right/left mouse buttons on Mouse, and
stop/play-pause on Media. Direct thumb keys and chord outputs on these layers
can be held for repeat or drag where the host supports it.

Caps Word moves from the old inner-thumb combo to **Nav + H**: hold Enter,
then tap H. Enter + Space is no longer a Caps Word combo, so Nav/Num holds do
not compete with it. Num + A gives `'`; Sym + A gives `"` on a US host layout.
Semicolon remains on Base; Shift + semicolon gives `:`.

Bootloader access follows REMAKE's guarded Miryoku binding: hold a layer thumb
or tertiary chord, then **double-tap the top-row pinkie on the same hand**
(Q for left-hand layers, P for right-hand layers). A single tap does not reset.

Clipboard uses Shift+Insert paste, Ctrl+Insert copy, Shift+Delete cut, HID Undo,
and **Ctrl+Y Redo**. Application support varies. This deliberately corrects
REMAKE's Again macro: the RMK engine interprets HID Again as repeat-last-key
even inside macros, rather than emitting the raw Redo usage. Auto Shift, layer
locks, alternate alpha layers, and unsupported RGB/external-power controls are
not included.

Home-row modifiers use buffered tap/hold, 200 ms permissive hold, and
same-hand tap filtering. Thumb layer-taps use 200 ms Normal mode without
hand filtering. Flow-tap and quick-tap repeat are disabled.

### Bluetooth profiles

There are **four host profiles, BT0–BT3**, matching REMAKE and Miryoku. Hold
both left thumbs for Media; the right bottom row, from inner to outer, is
**USB/BLE toggle, BT0, BT1, BT2, BT3**. Profile selection uses User0–User3;
the transport toggle uses User7. User IDs depend on the configured profile count.
The unused RGB and external-power positions stay unbound.

A profile selector switches on press. Holding it for five seconds starts
re-pairing, preserving the old bond until replacement pairing succeeds. This
prototype does not implement Miryoku's
Shift+profile clear or Shift+output force-USB semantics; Shift does not alter
these User actions. Next/previous profile and clear-current-bond remain available
for remapping in Vial, but are not assigned extra default positions.

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

On the `prototype` branch, firmware and host tests pin the same published RMK
Git revision in their Cargo manifests. A separate local RMK checkout is not
required. See [the experiment notebook](PROTOTYPE.md) for baseline revisions,
measurement procedures, and validation status.

This baseline migrates to current RMK main plus the rebased ordered tap-hold PR;
it is not a power optimization. RMK's Embassy patches are repeated in both
root Cargo manifests because dependency patches do not propagate to consumers.
Record the Urchin revision and pinned RMK revision for reproducibility. Changes
to RMK or Cargo features can clear stored settings and bonds, requiring host and split re-pairing.
Stored/Vial keymaps can override compiled defaults; back up custom mappings
before resetting them. Do not assume former BT4/BT5 bonds remain accessible.

## Vial

Connect the left (central) half over USB, keep the right half powered, and open
[Vial](https://get.vial.today/download/) or [Vial Web](https://vial.rocks/).
Select `rmk-urchin` to edit any of the seven layers. Changes are stored on the
keyboard automatically; use **File → Save current layout** to back them up.

`vial.json` describes all 34 physical keys, including the four thumbs. It is
embedded in the firmware by `build.rs`, so changes to the visual layout require
rebuilding and flashing. Use **File → Load dummy JSON** to preview `vial.json`
without flashing; dummy mode does not edit the connected keyboard.

When Vial prompts for unlock, hold **both outer thumbs (Enter + Space)**,
matrix positions `(3,3)` and `(3,6)`. The physical-presence lock stays enabled.
Named `THUMB` profiles are an RMK extension: replacing a thumb binding using
Vial's standard layer-tap encoding can lose its profile. Combos match actions,
so remapping their member keys also requires updating the corresponding combos.

## Displays and power

### Active-power prototype builds

The `prototype` branch has two independent, default-off experiments:

- `prototype-matrix`: cycle-counted matrix settling and 1 ms active scan gaps.
- `prototype-spim`: construct/drop the display SPIM driver around each I/O operation.

Build the comparisons from the same commit and Git-pinned RMK source:

```sh
cargo build --release --bins --locked
cargo build --release --bins --locked --features prototype-matrix
cargo build --release --bins --locked --features prototype-spim
cargo build --release --bins --locked --features prototype-matrix,prototype-spim
```

Each command replaces the release binaries; save or convert them before building
the next variant. For an explicitly selected flashable variant, use Cargo
directly rather than `cargo make uf2`, whose existing tasks do not pass these flags:

```sh
cargo objcopy --release --locked --bin central --features prototype-matrix,prototype-spim -- -O ihex prototype-both-central.hex
cargo hex-to-uf2 --input-path prototype-both-central.hex --output-path prototype-both-central.uf2 --family nrf52840
```

Replace `central` with `peripheral` for the other half. Select only the desired
feature for an isolated experiment, or omit `--features` for baseline, and name
the output accordingly. For initial **central-only** comparisons, keep the
peripheral on the baseline firmware. These are unmeasured experiments, not power
savings claims. See [the notebook](PROTOTYPE.md) for workload and validation details.

Each screen uses a compact dark layout with a battery icon and numeric
percentage, or `?` and `--%` when unknown. The bolt indicates local USB power,
not charging. The central shows the prominent uppercase layer and a compact
zero-based profile (`P0`, `P1`, etc.). Its static host icon distinguishes connected
(plain Bluetooth symbol), advertising (small squares and short arcs on both sides),
and disconnected (an X). The peripheral shows `OK` or `WAIT` for its split link.

![Dark screen previews: central connected, advertising and disconnected; peripheral connected and waiting](assets/screen-states.svg)

Pixel-accurate previews generated from the firmware renderer, enlarged for
readability. Battery values are examples, not measurements.

![Central screen previews for Base, Nav, Mouse, Media, Num, Sym and Fun](assets/screen-layers.svg)

Regenerate both screen SVGs from the actual firmware renderer with:

```sh
cargo test --manifest-path tests/host/Cargo.toml \
  --target "$(rustc -vV | sed -n 's/^host: //p')" --locked \
  export_screen_previews -- --ignored
```

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
tap/hold typing. Keymap checks resolve the actual `keyboard.toml` and exercise
all four physical thumb taps/holds, both tertiary chord taps/holds in either
press/release order, and all six opposite-hand tertiary actions. Firmware and
host tests use the same pinned RMK Git revision. Typing regressions also retain
capital I after Space without a stray A and early layer-thumb Quote without
a base-layer M.

On hardware, check fast rolls, deliberate modifier/layer holds, all thumb
chord taps/holds, mouse click/drag, guarded bootloader access, BT0–BT3,
USB/BLE switching, and wake after idle on each half. Native tests do not validate
physical timing or radio behavior.
