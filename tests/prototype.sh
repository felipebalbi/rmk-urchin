#!/usr/bin/env bash
set -euo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
[[ -f "$root/prototype" ]] || { echo 'Missing prototype build script' >&2; exit 1; }
sandbox=$(mktemp -d "${TMPDIR:-/tmp}/prototype-test.XXXXXX")
trap 'rm -rf -- "$sandbox"' EXIT
fixture="$sandbox/repo with spaces"
mkdir -p "$fixture" "$sandbox/bin"
cp "$root/prototype" "$fixture/prototype"
export PROTOTYPE_TEST_LOG="$sandbox/commands"
export PATH="$sandbox/bin:$PATH"

cat > "$sandbox/bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$PROTOTYPE_TEST_LOG"
case "$1" in
    build)
        [[ ${FAIL_STAGE:-} != build ]] || exit 23
        features=baseline
        target_dir=
        while [[ $# -gt 0 ]]; do
            case "$1" in
                --features) features=$2; shift 2 ;;
                --target-dir) target_dir=$2; shift 2 ;;
                --target) [[ $2 == thumbv7em-none-eabihf ]]; shift 2 ;;
                *) shift ;;
            esac
        done
        [[ -n $target_dir ]]
        mkdir -p "$target_dir/thumbv7em-none-eabihf/release"
        for role in central peripheral; do
            printf '%s %s%s\n' "$features" "$role" "${ARTIFACT_SUFFIX:-}" > "$target_dir/thumbv7em-none-eabihf/release/$role"
        done
        ;;
    hex-to-uf2)
        shift
        while [[ $# -gt 0 ]]; do
            case "$1" in
                --input-path) input=$2; shift 2 ;;
                --output-path) output=$2; shift 2 ;;
                --family) [[ $2 == nrf52840 ]]; shift 2 ;;
                *) exit 25 ;;
            esac
        done
        if [[ ${FAIL_STAGE:-} == conversion && $input == */peripheral.hex ]]; then
            exit 24
        fi
        cp "$input" "$output"
        ;;
    *) exit 26 ;;
esac
STUB
cat > "$sandbox/bin/rust-objcopy" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
[[ $1 == -O && $2 == ihex ]]
cp "$3" "$4"
STUB
cat > "$sandbox/bin/git" <<'STUB'
#!/usr/bin/env bash
case "$1" in
    rev-parse) printf '%s\n' test-commit ;;
    status) ;;
    *) exit 27 ;;
esac
STUB
chmod +x "$sandbox/bin/"*

# Build from outside the fixture; paths with spaces must not split arguments.
cd "$sandbox"
for variant in baseline matrix spim both; do
    case "$variant" in
        baseline) expected=baseline ;;
        matrix) expected=prototype-matrix ;;
        spim) expected=prototype-spim ;;
        both) expected=prototype-matrix,prototype-spim ;;
    esac
    : > "$PROTOTYPE_TEST_LOG"
    "$BASH" "$fixture/prototype" "$variant" > "$sandbox/output"
    output="$fixture/target/power-prototypes/$variant"
    for role in central peripheral; do
        for extension in elf hex uf2; do
            [[ $(cat "$output/$role.$extension") == "$expected $role" ]]
        done
    done
    (cd "$output" && sha256sum -c SHA256SUMS > /dev/null)
    grep -Fxq "variant=$variant" "$output/BUILD.txt"
    grep -Fxq 'commit=test-commit' "$output/BUILD.txt"
    [[ $(grep -c '^build ' "$PROTOTYPE_TEST_LOG") == 1 ]]
    grep -q -- '--locked' "$PROTOTYPE_TEST_LOG"
    grep -q -- '--no-default-features' "$PROTOTYPE_TEST_LOG"
    if [[ $variant == baseline ]]; then
        ! grep -q -- '--features' "$PROTOTYPE_TEST_LOG"
    fi
done

for bad in missing unknown extra; do
    : > "$PROTOTYPE_TEST_LOG"
    case "$bad" in
        missing) args=() ;;
        unknown) args=(unknown) ;;
        extra) args=(baseline extra) ;;
    esac
    if "$BASH" "$fixture/prototype" "${args[@]}" > "$sandbox/output" 2>&1; then
        echo "Accepted invalid invocation: $bad" >&2
        exit 1
    fi
    [[ ! -s "$PROTOTYPE_TEST_LOG" ]]
done

cp "$fixture/target/power-prototypes/both/SHA256SUMS" "$sandbox/before"
for failure in build conversion; do
    if FAIL_STAGE=$failure ARTIFACT_SUFFIX=-changed "$BASH" "$fixture/prototype" both > "$sandbox/output" 2>&1; then
        echo "Ignored $failure failure" >&2
        exit 1
    fi
    cmp "$sandbox/before" "$fixture/target/power-prototypes/both/SHA256SUMS"
    (cd "$fixture/target/power-prototypes/both" && sha256sum -c SHA256SUMS > /dev/null)
    ! grep -q 'Ready:' "$sandbox/output"
done

printf '%s\n' 'Prototype script tests passed (four variants, invalid arguments, failure preservation).'
