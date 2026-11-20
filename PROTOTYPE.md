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
- Owner-amended Urchin migration baseline: `e1f34c35007b6824f2589d1d3cb82fe38c18a7ac`.

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
`0d9c302094dfc2aacc62263d09b19f7013e18611` in both Cargo manifests. No sibling
checkout is required. Record the Urchin commit, dirty status, and exact RMK pin
with each measurement. Publish each future RMK experiment commit before updating
the pin; do not use a moving branch reference for measurement checkpoints.

The first migration commit (`d1c18bc`) used sibling path dependencies. This was
corrected to a Git pin at the owner's request so a checkout on the measurement
machine resolves the same published source without local workspace assumptions.

## Active experiments

Two independent Cargo features select the experiments; both are off by default.

| Variant | Cargo features | Matrix policy | Display SPIM lifetime |
|---|---|---|---|
| Baseline | none | Existing async line wait and unpaced active rescans | Retained |
| Matrix | `prototype-matrix` | Cycle-counted short settling; 1 ms gap between active scans | Retained |
| SPIM | `prototype-spim` | Baseline | Construct/drop around each I/O operation |
| Both | `prototype-matrix,prototype-spim` | Experimental | Experimental |

The matrix flag forwards RMK's `prototype_matrix_scan` feature. Its settling
wait is `cortex_m::asm::delay(64)` on the fixed 64 MHz nRF52840: nominally 1 µs
without scheduling a sub-tick RTC timer. This is not a measured/calibrated
electrical delay. The 1 ms gap after a complete active scan is what bounds
polling frequency. GPIO-only idle wake and the 5 ms debounce configuration remain.

The SPIM flag retains only peripheral/pin ownership tokens and configuration
between operations. Each operation constructs the HAL driver, uses the existing
DMA cancellation guard, flushes, and drops the driver. HAL drop disables SPIM;
then SCK/MOSI are parked low. Cancellation must stop DMA before its buffers and
the driver are dropped. No display driver remains enabled between panel ticks
or while the screen sleeps. Reinitialization overhead may offset savings;
measure rather than assuming this experiment is better.

Neither flag changes BLE intervals, display setup/hold waits, image policy,
16 ms COM cadence, or regulator/clock settings. These are board prototypes,
not production defaults or demonstrated current reductions.

### Build and flash comparisons

From the Urchin repository, build and convert each named variant:

```sh
./prototype baseline
./prototype matrix
./prototype spim
./prototype both
```

All four builds use the same pinned RMK revision; the baseline does not need a
branch checkout or a dependency edit. The script explicitly disables defaults,
selects only the variant's experiment flags, builds both roles with `--locked`,
and converts the resulting ELF files through HEX to UF2. It fixes the target and
target directory so unrelated Cargo environment settings cannot select stale
release files. It never flashes a board.

Each `target/power-prototypes/<variant>/` directory contains both roles' ELF,
HEX and UF2 files, a `SHA256SUMS` file, and `BUILD.txt` recording the variant,
features, target, Git commit and tracked working-tree status. Both conversions
must succeed before the new output set is published. Contract checks for the
script can be run with `bash tests/prototype.sh` without compiling firmware.

For initial central-current attribution, keep the peripheral on baseline while
changing only the central image. Then optionally repeat with matching variants
on both halves and record that distinction. Changing Cargo features can reset
RMK settings/bonds: back up mappings, re-pair if required, and wait for connection
parameter negotiation before measuring. Do not compare initial advertising or
pairing transients with an established connection.

Keep distinct files and checksums for every role/variant. Do not flash whichever
binary happens to remain in the shared release directory after the last build.

## Experiment sequence

- [x] Rebase the published PR onto main and create both prototype branches.
- [x] Build both migrated firmware targets and run the board's host checks.
- [ ] Resolve the broader RMK feature-matrix validation gap separately.
- [x] Prototype paced active matrix scans with short settling delays.
- [x] Prototype SPIM construction/drop between display operations.
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

### Active-experiment validation

- RMK matrix feature: **582 tests passed**. Feature-disabled baseline: **579 passed**.
- The paced-scan regression failed before implementation (79 reads over 8 ms),
  then passed with bounded release polling. A separately reproduced immediate
  GPIO-error spin was corrected with a cooperative yield after idle waits.
- Each of the four Urchin feature combinations: **72 host tests passed,
  1 intentionally ignored** (preview exporter).
- All four combinations built **both central and peripheral release firmware**
  from the published RMK pin. Subsequent experiment builds used `--locked`.
- Source review approved the matrix timing/state handling and SPIM
  ownership/drop/cancellation paths. The shared six-word timer-queue dependency
  remains unified. Host models do not validate actual pins, IRQ timing or power.
- The broader Windows BLE/passkey crypto-linker gap below remains unresolved;
  no retries or weakened checks were introduced to bypass it.
- The Bash wrapper was exercised for all four variants under Git Bash on
  Windows, including real firmware builds and UF2 conversion. All 24 output
  checksums passed. Its isolated command tests cover feature selection, both
  roles, paths with spaces, invalid arguments, and retaining the last complete
  output set when a build or the second conversion fails.

Local ELF, HEX and UF2 files were generated for both roles in:

```text
target/power-prototypes/baseline/
target/power-prototypes/matrix/
target/power-prototypes/spim/
target/power-prototypes/both/
```

Each directory contains `central.uf2` and `peripheral.uf2` plus their ELF/HEX
inputs. These generated files are ignored by Git, not uploaded with the branch.
Per-variant `SHA256SUMS` and `BUILD.txt` are the current build records. The earlier
combined `target/power-prototypes/checksums.csv` is a historical snapshot, not
updated by the single-variant script. No device has been flashed by this session.
The previously recorded migration ELF hashes below identify older builds, not
these experiment images.

Initial manually generated UF2 fingerprints from the local Rust 1.100.0 build
(historical snapshot before the wrapper, not a cross-machine reproducible-build
guarantee; use each directory's current `SHA256SUMS` after running the script):

| Variant | Role | SHA-256 |
|---|---|---|
| baseline | central | `a00eb6555998c5a65c2e858b226c031d14450d4740b801d9b381b5278b588421` |
| baseline | peripheral | `b912f571c47d3a183298de2062c977f853b95ae0af955fe1f46967f59d0edc1e` |
| matrix | central | `517f7a93edba5f452328a8f949bdb23e0772e49e58b6ae2557c99006958b6d85` |
| matrix | peripheral | `b4d1e8f919b95f4cd38552e6cc1d2e811ba0a65bb567d058b679663378b1d318` |
| spim | central | `ede790c7f13008eb3e2be73fa7879ea1fae5aa627aacadd73bb2d0314018a97d` |
| spim | peripheral | `84d5345116835e20f384c9da5f11e4f950b0e56882fb57f2a480e40663388efc` |
| both | central | `819b801a343a1c128a119532303e028de1917264c1106f250407345d0adfeabc` |
| both | peripheral | `554c6e3afed71bab8e38bcdcb904a2d7080fc8bbdd1c2737d200eba68751b1f7` |

### Historical migration validation

Software migration validation on Windows/MSVC, Rust 1.100.0, before the experiments:

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

The owner requested proceeding with the board experiments while preserving
this known validation gap. Debug and release builds have already failed with
the same missing-symbol class; do not repeat optimization-flag guesses or
weaken the affected tests. The current hypothesis is a Windows
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
