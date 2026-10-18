# RMK 

RMK is a feature-rich and easy-to-use keyboard firmware.

## uf2 support

If you’re using the Adafruit_nRF52_Bootloader (pre-installed on the nice!nano), you’re in luck! This bootloader supports the .uf2 firmware format, which eliminates the need for a debugging probe to flash your firmware. RMK uses the `cargo-make` tool to generate .uf2 firmware, with the generation process defined in the `Makefile.toml`.

Follow these steps to generate and flash the .uf2 firmware with RMK:

1. Get `cargo-make` tool:
   ```shell
   cargo install --force cargo-make
   ```
2. Compile RMK and generates .uf2 firmware:
   ```shell
   cargo make uf2 --release
   ```
3. Flash

   - Put your board into bootloader mode. A USB drive will appear on your computer.
   - Drag and drop the generated .uf2 firmware file onto the USB drive. The RMK firmware will be automatically flashed onto your microcontroller.

   For additional details on entering bootloader mode and flashing firmware, refer to the [nice!nano documentation](https://nicekeyboards.com/docs/nice-nano/getting-started#flashing-firmware-and-bootloaders)

### Tips for nRF52840

Most nice!nano compatible boards have bootloader with SoftDevice pre-flashed. Since v0.7.x, RMK will remove old SoftDevice Bluetooth stack and replace it with its own. So if you want to rollback to v0.6.x, or switch to firmwares that use SoftDevice stack(for example, zmk), you will need to [re-flash the bootloader](https://nicekeyboards.com/docs/nice-nano/troubleshooting#my-nicenano-seems-to-be-acting-up-and-i-want-to-re-flash-the-bootloader).

### Additional notes

RMK defaults to USB-priority mode if a USB cable is connected. After flashing, remember to disconnect the USB cable, or [switch to BLE-priority mode](https://rmk.rs/docs/features/wireless.html#multiple-profile-support) by pressing User11(Switch Output) key.

## Host regression tests

Each half shows a horizontal battery icon with ten segments (one per complete
10%), or a `?` when its battery level is unavailable. The bolt beside `BATTERY`
indicates local USB power, not charging activity.
The central half shows its zero-based Bluetooth profile number in large type at
the bottom right; the peripheral does not show a host profile.

Run `cargo make test`, or without cargo-make:

```sh
cargo test --manifest-path tests/host/Cargo.toml \
  --target "$(rustc -vV | sed -n 's/^host: //p')" --locked
```

The explicit host target overrides this repository's default ARM target. Run
from the repository root so `.cargo/config.toml` supplies the real
`keyboard.toml` to RMK's configuration build. The separate test crate keeps
host-only features and dependencies out of firmware builds.

These checks compile the firmware's display modules directly and use the same
selected RMK source as the firmware, including the pinned fork override below.
Coverage includes framebuffer
rotation/framing, role-specific Bluetooth icons and advertising animation,
panel refresh scheduling and recovery, DMA cancellation ordering, startup RMK
events, local USB power indicators, and configured 5 ms debounce behavior.
Representative six-key typing fixtures also load the project's morse settings
through RMK's configuration resolver and exercise real tap/hold processing.
`embedded-hal-mock` checks queued SPI/GPIO operations; stateful test doubles
model pending transfers, power loss, injected failures, and elapsed time.

They do not validate physical radio, GPIO, or display timing. The earlier spike's
full key/chord scenarios require RMK's unpublished simulator and are not part
of this suite.

### Desired-behavior regressions

Two ordinary tests in `tests/host/src/typing_checks.rs` pin the desired behavior
that failed against the upstream comparison build and its flow-tap policy:

- `capital_i_after_space_does_not_emit_a_from_the_shift_key` replays the captured
  Space / A-Shift / I timing and expects Space followed by shifted I, without A.
- `early_right_thumb_quote_chord_does_not_emit_base_layer_m` expects the M position
  to produce shifted Quote when chorded with the held Right thumb before its
  timeout. This is a representative reproduction of the reported quote failure,
  using timing from the captured thumb/base-key sequence, not an exact quote trace.

Controls cover capital I after idle and Quote after the thumb hold timeout. The
two tests specifically pinning forced flow-tap remain under an explicit legacy
policy fixture, with their original assertions. No reproducer is ignored or
marked as an expected panic. These are representative native checks, not proof
of physical typing or radio behavior.

## Tap/hold tuning

Urchin opts into RMK's experimental `buffered_tap_hold` feature and profile flag.
Flow-tap is **disabled**; its retained 120 ms prior-idle setting is inactive.
Home-row modifiers use 200 ms permissive hold and release-based hand filtering:
an opposite-hand nested release holds, a same-hand nested release taps, and a
same-hand press alone can still hold through the term. Thumb layer-taps use
Normal mode without unilateral filtering, inheriting buffering and the 200 ms term.
Quick-tap repeat remains explicitly disabled.

Following presses and releases wait behind an undecided tap/hold. The owner
commits first, then captured keys execute on the selected layer with their original
logical times. Expect bounded decision latency, commonly up to the 200 ms term;
full Morse/tap-dance owners remain legacy. The 32-event FIFO degrades oldest owners
to taps under exhaustion, and legacy ownership exhaustion can degrade a fresh
press to a balanced tap. This avoids stuck actions, not incorrect typing intent:
a degraded Backspace tap can delete text.

This follows the selected ZMK home-row/thumb settings, not full ZMK parity. RMK
still releases combo output on the last member and resolves events at or after
the tapping deadline as timeouts. Mappings, pins, layers and thumb combos are
unchanged. The configured central idle timeout is 30 seconds.

## Screen sleep and wake

Both screens consume RMK's sleep/wake events, including the existing split-link
sleep notification. On sleep, a screen finishes any in-flight transaction, sends
a final clear, and cuts its nice!nano v2 external 3.3V supply. Periodic display
updates and maintenance stop while asleep. Status events still update RAM so
battery, profile, layer and connection information is current after waking.

On wake, the supply is enabled, the existing 50 ms settling interval is observed,
and the panel is reinitialized and fully redrawn. The normal 16 ms awake cadence
then resumes without replaying missed ticks. The nice!nano's switched supply may
also power other external accessories; those lose power during screen sleep too.
This is display power management, not MCU System OFF or a fix for active-display
SPI power consumption.

## Fork source and optional USB logging

Firmware and host checks pin upstream `main` at
[`535cd7191338bd6fec656dfd4a4b36c1f159736d`](https://github.com/rmk-rs/rmk/commit/535cd7191338bd6fec656dfd4a4b36c1f159736d)
(2026-10-16; upstream HEAD checked on 2026-10-17) as a comparison reference.
`.cargo/config.toml` patches that Git source to the published buffered fork at
[`5f0ab05292f6d757d25c0491071c6c8190318831`](https://github.com/felipebalbi/rmk/commit/5f0ab05292f6d757d25c0491071c6c8190318831).
Firmware and host checks both enable `buffered_tap_hold` and resolve this exact
revision. No sibling checkout is required, and the build does not follow a moving
branch. The upstream SHA alone is the comparison identity, not the selected source.
Compatibility updates move each half's battery ADC into its own `.battery` table,
use bare processor registration, and clone the USB device configuration before
giving the remaining configuration to BLE. Key mappings and the display's custom
awake refresh cadence are preserved; the selected tap/hold policy is described above.

**Storage warning:** RMK hashes its commit and Cargo features into its storage
identity. A changed build can erase stored settings, host bonds and split peers,
requiring re-pairing. Changing logging features can trigger another reset. With
a matching schema, persisted profile settings still override compiled defaults.

USB CDC logging is currently **disabled** on both halves for power testing.
USB keyboard/HID, Vial, storage, host locking and the watchdog remain enabled.
RMK's `defmt` feature is disabled; RTT remains linked for panic reporting and
defmt-only dependencies. No serial logger is exposed by the normal build.

Build and flash both halves using the normal UF2 procedure:

```sh
cargo make test
cargo make uf2 --release
```

### Temporarily enable USB logs

For debugging, add `"usb_log"` back to the RMK dependency's feature list in
`Cargo.toml`, then rebuild with the same UF2 command. The retained application
`log` configuration caps output at Debug; `DEFMT_LOG` does not control this logger.
Changing the RMK feature set may reset storage and require re-pairing again.

After flashing that logging-enabled build, connect the **central** half using a USB data cable. Find its
serial device with `ls -l /dev/serial/by-id/` (usually `/dev/ttyACM0`). If both
halves are connected, identify each port separately; they produce separate logs.
Open a serial monitor at 115200 baud, or capture directly on Linux:

```sh
umask 077
stty -F /dev/ttyACM0 115200 raw -echo
cat /dev/ttyACM0 | tee /tmp/opencode/urchin-central-usb.log
```

Replace the port as needed, type a short non-sensitive reproduction, and press
Ctrl-C to stop capture. If access is denied, use your system's serial-port access
policy or run the serial-reader commands with the necessary privilege; do not make
all device nodes world-readable. Logs can contain key positions and actions, so
review them before sharing.

The upstream logger formats lines as `[<uptime>ms <level>] <message>`. Logs before
the logger/USB connection is available may be absent; this is a bounded text
logger, not a loss-accounted telemetry stream. USB attachment keeps RMK's existing
USB/BLE output selection behavior, and logging can affect timing. First establish
that the port emits messages, then compare slow typing with the fast rolls that
misbehave. No custom HID telemetry is enabled in this build.

### Hardware comparison

Keep the previous upstream images separately, and flash matching central and
peripheral images yourself using the UF2 procedure above. Compare slow and normal
typing with a short, non-sensitive sequence:

- Space followed by an intentional capital I: no extra A.
- Early and late Right-thumb quotes at the M position: shifted Quote, not M.
- Two/three-letter rolls, especially ASQ, and ordinary Tab/Backspace taps.
- Opposite-hand modifiers, same-hand rolls and deliberately held same-hand chords.
- All Escape/Delete/Caps Word thumb combinations and both-thumb tri-layer access.
- Idle for more than 30 seconds, then check wake and split reconnection separately.

USB logging is off in the selected build. If it is temporarily enabled for a
short diagnosis, compare again without it before attributing timing changes
solely to buffering. Native tests and valid UF2 files do not replace physical
typing, screen power, wake and radio checks.
