# Active-power prototype notebook

This branch prepares controlled power experiments for a future balbi.sh article.
It is not a claim that any experiment reduces measured current.

## Starting points

- Urchin before migration: `2026180e53c0b6aef0cf7b0679417c0cb5d1bf37`.
- Original firmware RMK pin: `18300680e2f16046643b9f8151abae891df998a4`.
- Ordered tap-hold PR: <https://github.com/rmk-rs/rmk/pull/1347>.
- Published PR tip selected for rebasing: `534e7875d6f2ee0b2c93608f9660eeb5f5bbc186`.
- Upstream main used for the rebase: `e4e68e231c35d31ae43092ed894280b311828a89`.
- Rebased PR / initial RMK prototype: `e9e726304cb34aa36583f900be1c54da41fe0fde`.

The five PR commits were rebased, not squashed. The one manual conflict was in
combo dispatch: retain the PR's ordered-input implementation while using
upstream's `MAX_COMBOS` capacity name. Independent review found no integration
issues. The obsolete local tip and original published PR tip remain available
under `backup/held-buffer-tap-hold-local-f9efbdbb5` and
`backup/held-buffer-tap-hold-pr-534e7875d` in the local RMK repository. The rebased
PR branch and both prototype branches were subsequently pushed at the owner's request.

The owner's existing **central** measurement is approximately **1–1.3 mA while
typing**. Supply voltage, measurement point, instrument, typing workload, and
awake-idle current were not recorded here. Do not compare it quantitatively
with new measurements until those conditions are established.

Firmware and host tests pin the published RMK Git revision
`e9e726304cb34aa36583f900be1c54da41fe0fde` in both Cargo manifests. No sibling
checkout is required. Record the Urchin commit, dirty status, and exact RMK pin
with each measurement. Publish each future RMK experiment commit before updating
the pin; do not use a moving branch reference for measurement checkpoints.

The first migration commit (`d1c18bc`) used sibling path dependencies. This was
corrected to a Git pin at the owner's request so a checkout on the measurement
machine resolves the same published source without local workspace assumptions.

## Experiment sequence

- [x] Rebase the published PR onto main and create both prototype branches.
- [x] Build both migrated firmware targets and run the board's host checks.
- [ ] Resolve the broader RMK feature-matrix validation gap; do not change power policy yet.
- [ ] Prototype paced active matrix scans with short settling delays.
- [ ] Prototype SPIM disabling between completed display transactions.
- [ ] Prototype short display setup/hold delays without RTC-backed async waits.
- [ ] Evaluate BLE connection intervals separately, with explicit latency tradeoffs.
- [ ] Capture hardware current traces and typing/wake acceptance for each checkpoint.

Keep one mechanism per experiment commit. Record its parent baseline and exact
build command. Measure isolated variants against the migration baseline as well
as any cumulative build; otherwise interactions can be mistaken for individual
savings. Do not compare only the old firmware with a build containing every change.

## What remains unchanged in the migration baseline

- External LF crystal and declared 50 ppm accuracy.
- Both DC/DC regulator settings and 3.3 V REG0 output.
- Existing RMK scan timing, BLE interval policy, and 5 ms debounce.
- Display SPI at 1 MHz, SPI2 ownership/cancellation protection, and 16 ms inversion cadence.
- Changed-field rendering, 100 ms image rate limit, and screen power-off on sleep.
- Release optimization settings: size optimization, fat LTO, one codegen unit.

Updating RMK is a separate baseline change: upstream API/configuration migration
and the published PR's default ordered tap-hold replace the old opt-in engine.
It must not be attributed to an active-power optimization. Dependency/configuration
changes can invalidate stored settings and bonds; back up mappings before flashing.

## Measurement worksheet

For every run record:

- Urchin and RMK commit IDs, dirty status, compiler version, build command, and firmware checksum.
- Central/peripheral role; peer firmware revision; USB disconnected or enumerated.
- Supply voltage, battery versus regulated-rail measurement point, instrument, and sample rate.
- Negotiated host/split connection intervals, PHY, host adapter, distance, and other connected profiles.
- Screen powered state and whether the workload changes its visible content.
- Mean current over a stated duration, baseline floor, peaks, and integrated charge per repeated action.
- Typing correctness, release correctness, scan latency, and sleep/wake behavior.

Use the same hardware and workload for paired measurements. Disconnect the debug
probe and disable diagnostic output for final current measurements. Any GPIO
instrumentation must be recorded and checked for its own power cost.

Suggested workloads:

1. Awake, connected, screen on, no held keys; measure before the 30 s idle timeout.
2. One ordinary local base-layer key held, avoiding repeated visible state changes.
3. One ordinary remote key held, separating local scanning from split traffic.
4. Repeatable overlapping base-layer typing.
5. Repeated layer-thumb use, exercising changed-row display writes.
6. Repeat with the screen safely shut down but the keyboard still awake.
7. Idle through sleep, then type immediately on both halves to check wake and releases.

Use the existing clear/power-off sequence for a screen-off variant. Never leave
the LCD powered indefinitely without the required inversion maintenance.

## Reference constraints

- Nordic nRF52840 Product Specification v1.11, §6.25.5: stop/disable SPIM when not needed;
  if STOP is requested, wait for STOPPED before disabling.
  <https://mm.digikey.com/Volume0/opasdata/d220001/medias/docus/6469/NRF52840-DK.pdf>
- Sharp LS011B7DH03, LD-29Y03, §6-3: serial-inversion frame frequency 57–70 Hz;
  serial clock maximum 1.1 MHz. Preserve the existing inversion policy.
  <https://www.beyondinfinite.com/lcd/Library/Sharp/LS011B7DH03.pdf>
- At 32,768 Hz, one Embassy time tick is approximately 30.5 µs. A nominal 1 µs
  asynchronous timer is not a calibrated 1 µs settling delay. Hardware timing and
  power still need measurement; simulator success cannot establish either.

## Validation record

Hardware measurements: **pending the measurement board**.

Software migration validation on Windows/MSVC, Rust 1.100.0:

- RMK `cargo nextest run --manifest-path rmk/Cargo.toml --config-file .config/nextest.toml --no-default-features`:
  **579 passed** (unit and integration tests).
- Urchin `cargo nextest run --manifest-path tests/host/Cargo.toml --target x86_64-pc-windows-msvc --locked`:
  **69 passed, 1 intentionally ignored** (screen-preview exporter).
- Urchin `cargo build --release --bins --locked`: **both central and peripheral built**.
  Cargo reports an existing dependency future-incompatibility warning for `proc-macro-error2`.
- Firmware `cargo tree --locked --offline -i embassy-executor-timer-queue -e features --depth 2`:
  exactly one git `0.1.1` package, shared by the executor and timer queue,
  with `timer-item-size-6-words` enabled.
- Broader RMK matrix: **blocked**, not accepted as passing. The
  `rynk,_ble,split,async_matrix,storage` row fails at link time in both debug and
  release on Windows/MSVC, with 136 unresolved `_embassy_crypto_*` symbols.
  `vial,host_lock,_no_usb,steno,passkey_entry` also reaches the same crypto link
  failure. These tests did not execute. No test expectations, feature coverage,
  or cryptographic implementations were weakened to bypass the failures.

Stop further power-policy changes until this validation gap is resolved. Debug
and release builds have already failed with the same missing-symbol class;
do not repeat optimization-flag guesses. The current hypothesis is a Windows
linking interaction with Embassy's link-time crypto drivers, but attribution
to upstream requires an isolated reproduction on unmodified main. A supported
Linux environment or a confirmed native-linker fix is still needed for those checks.

RMK requires `cargo-nextest` because its mock clock is process-global. A
preliminary plain `cargo test --lib` build compiled, but execution was rejected
by the repository's nextest guard; that run is not a passing baseline and does
not establish a firmware regression.

### Published Git-pin build

After replacing the local paths with the published Git revision, both firmware
binaries were rebuilt with `--locked` and the host suite reran: **69 passed,
1 intentionally ignored**. Both lockfiles resolve all four RMK crates from the
same Git revision. The firmware still has exactly one patched timer-queue package
with the shared six-word storage feature.

Local Git-pin build ELF fingerprints (SHA-256):

- `target/thumbv7em-none-eabihf/release/central`:
  `c264f0ef8402963916262d24b341e97261041e3c8c98f9041418862bb1acc698`
- `target/thumbv7em-none-eabihf/release/peripheral`:
  `7fb2a4930d061e427e330d783b42f50855cfca3314cba129135c6ce1607e543e`

### Earlier path-dependency build

Initial path-dependency migration-build ELF fingerprints (SHA-256, not the
pre-existing HEX/UF2 files; superseded by the Git-pin artifacts above):

- `target/thumbv7em-none-eabihf/release/central`:
  `eac5de8cf1edada2f4a0cab5afc7808a06bc4bf031464043c806906cbfb67d78`
- `target/thumbv7em-none-eabihf/release/peripheral`:
  `9b17ae6939983ba57222f5bdac6faaccceb4b1f8790eb9defea8dea1f6740cc6`

These are fingerprints of this machine's artifacts, not a cross-machine
reproducible-build claim. No new HEX/UF2 files were generated or flashed.

### Rejected intermediate build: split timer-queue storage

Review found that the first successful firmware link retained the registry
`embassy-executor-timer-queue 0.1.0` for the executor, while patched Embassy's
integrated queue used git `0.1.1`. Their size features were separate: the executor
allocated zero timer words, but the queue accessed six words through a shared
extern accessor. That is a memory-safety defect, not merely duplicate code size.
Host tests use a generic queue and do not detect this firmware-only mismatch.

Resolved with the targeted lockfile update
`cargo update --offline -p embassy-executor-timer-queue@0.1.0 --precise 0.1.1`.
The feature graph now shares one six-word allocation between both consumers,
and both firmware binaries were rebuilt. The earlier ELF fingerprints beginning
`b00a5555` / `c46abbff` are unsafe, superseded artifacts, not valid checkpoints.
Do not flash old generated files on the assumption that a successful link proves
the timer-storage contract. The corrected path-build fingerprints appear above;
they do not identify later Git-pin build artifacts.
