#!/usr/bin/env bash
# API coverage with the normal public API, no tarpaulin cfg or percentage gate.
# Requires existing Rust 1.96.1, Python 3, and compatible LLVM tools; installs nothing.
# Usage: PATH=/path/to/1.96.1/bin:$PATH scripts/coverage.sh [--offline]
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

cargo_args=(--locked)
case "${1:-}" in
    '') ;;
    --offline) cargo_args+=(--offline) ;;
    *) echo 'Usage: scripts/coverage.sh [--offline]' >&2; exit 2 ;;
esac
[[ $# -le 1 ]] || { echo 'Too many arguments' >&2; exit 2; }
command -v python3 >/dev/null
compiler=$(rustc --version)
[[ "$compiler" == 'rustc 1.96.1 '* ]] || {
    echo "Expected Rust 1.96.1 on PATH, found: $compiler (no automatic installation)" >&2
    exit 1
}
verbose=$(rustc -vV)
llvm_version=$(sed -n 's/^LLVM version: //p' <<< "$verbose")
host=$(sed -n 's/^host: //p' <<< "$verbose")
tool_dir="$(rustc --print sysroot)/lib/rustlib/$host/bin"
find_tool() {
    local name=$1 path version
    if [[ -x "$tool_dir/$name" ]]; then
        path="$tool_dir/$name"
    else
        path=$(command -v "$name") || {
            echo "Missing $name: provide Rust 1.96.1 llvm-tools-preview or system LLVM ${llvm_version%%.*} tools" >&2
            return 1
        }
    fi
    version=$("$path" --version | sed -nE 's/.*LLVM version ([0-9]+\.[0-9]+\.[0-9]+).*/\1/p')
    [[ -n "$version" && "${version%%.*}" == "${llvm_version%%.*}" ]] || {
        echo "Incompatible $path ($version): rustc uses LLVM $llvm_version" >&2
        return 1
    }
    printf '%s\n' "$path"
}
profdata=$(find_tool llvm-profdata)
cov=$(find_tool llvm-cov)

# A fresh directory prevents stale binaries/profiles from contaminating the report.
# Everything generated stays under ignored target/, including build artifacts.
mkdir -p target/coverage
output=$(mktemp -d "$PWD/target/coverage/run.XXXXXX")
export CARGO_TARGET_DIR="$PWD/target/coverage/build"
# Do not inherit cfg flags that might silently change the measured API.
unset CARGO_ENCODED_RUSTFLAGS
export RUSTFLAGS='-C instrument-coverage -C link-dead-code'
export LLVM_PROFILE_FILE="$output/%m-%p.profraw"
printf '%s\n' "$verbose" > "$output/toolchain.txt"
"$profdata" --version >> "$output/toolchain.txt"
"$cov" --version >> "$output/toolchain.txt"

# Compile the non-test API too: unit tests use a loopback-only HTTP facade.
cargo build -p bourso_api "${cargo_args[@]}"
cargo test -p bourso_api --all-targets --no-run "${cargo_args[@]}" --message-format=json > "$output/artifacts.jsonl"
cargo test -p bourso_api --all-targets "${cargo_args[@]}" 2>&1 | tee "$output/tests.txt"
python3 - "$output/artifacts.jsonl" "$output/objects.txt" <<'PY'
import json, sys
objects = set()
with open(sys.argv[1]) as artifacts:
    for line in artifacts:
        item = json.loads(line)
        if (item.get('reason') == 'compiler-artifact'
                and item.get('profile', {}).get('test') and item.get('executable')):
            objects.add(item['executable'])
if not objects:
    raise SystemExit('No test executables found')
with open(sys.argv[2], 'w') as output:
    output.write('\n'.join(sorted(objects)) + '\n')
PY
shopt -s nullglob
profiles=("$output"/*.profraw)
[[ ${#profiles[@]} -gt 0 ]] || { echo 'No coverage profiles produced' >&2; exit 1; }
"$profdata" merge -sparse "${profiles[@]}" -o "$output/coverage.profdata"
mapfile -t objects < "$output/objects.txt"
object_args=("${objects[0]}")
for object in "${objects[@]:1}"; do object_args+=(-object "$object"); done
mapfile -t sources < <(find "$PWD/src/bourso_api/src" -name '*.rs' -type f | sort)
common=("${object_args[@]}" -instr-profile="$output/coverage.profdata")
"$cov" report "${common[@]}" "${sources[@]}" | tee "$output/summary.txt"
"$cov" report "${common[@]}" -show-functions "${sources[@]}" > "$output/functions.txt"
"$cov" export "${common[@]}" -format=lcov "${sources[@]}" > "$output/lcov.info"
"$cov" show "${common[@]}" -format=html -output-dir="$output/html" "${sources[@]}"
cat > "$output/SCOPE.txt" <<'SCOPE'
Scope: bourso_api package, all-targets unit tests, all Rust sources under
src/bourso_api/src, including test/support modules. Dependencies and the CLI are
outside this API report. No API functions or source files are excluded, and no
percentage threshold is asserted. Unexecuted functions remain visible where LLVM
emits coverage mappings. Macro expansion/compiler-generated coverage follows LLVM.

The same public functions compile in normal and instrumented builds. Unit tests
replace the private reqwest field with a cfg(test) loopback-only HTTP facade;
financial/authentication method bodies are shared, but the production transport
configuration is not exercised by these fixtures. Mock results do not establish
live bank compatibility. Tests use no bank credentials or real bank requests.
Rust and LLVM versions are in toolchain.txt; matching LLVM majors are required and
actual profile merge/report generation must succeed (minor versions may differ).
SCOPE
printf '\nCoverage artifacts: %s\n' "$output"
if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
    printf 'report_dir=%s\n' "$output" >> "$GITHUB_OUTPUT"
fi
