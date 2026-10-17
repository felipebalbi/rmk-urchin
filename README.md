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

These checks compile the firmware's display modules directly and use published
RMK 0.9.0, not a vendored or patched checkout. Coverage includes framebuffer
rotation/framing, role-specific Bluetooth icons and advertising animation,
panel refresh scheduling and recovery, DMA cancellation ordering, startup RMK
events, local USB power indicators, and configured 5 ms debounce behavior.
`embedded-hal-mock` checks queued SPI/GPIO operations; stateful test doubles
model pending transfers, power loss, injected failures, and elapsed time.

They do not validate physical radio, GPIO, or display timing. The earlier spike's
full key/chord scenarios require RMK's unpublished simulator and are not part
of this suite.
