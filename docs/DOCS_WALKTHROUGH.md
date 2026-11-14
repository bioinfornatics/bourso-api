# Local README walkthrough (DOCS-1)

This is a verification recipe and bounded evidence report, not task closure or
bank compatibility evidence. It accompanies `tests/docs_walkthrough.rs` for
`bourso-y3u.31`. The criterion was read from the existing task export
`/tmp/bourso-after-roadmap.json` on 2026-11-13; that temporary snapshot is not the
live tracker. The coordinator must check current criteria before acceptance.

## Scope

Goal: exercise the README's synthetic configuration command and the real CLI's
quote option parser, without a bank account, password, public-market request or
financial submission. The test uses Cargo's actual built binary, not a recreated
Clap command. Only the executable path changes from the README's release path to
`CARGO_BIN_EXE_bourso-cli`; the example arguments are checked against README lines.

Non-goals: release-install verification, financial operations, live quote success,
password/MFA interaction, Windows/macOS execution, DCA automation and production
readiness. No production parser or dispatch seam was changed by this work.

## What each check establishes

| Check | Observable evidence / limits |
| --- | --- |
| `readme_config_really_saves_synthetic_id_and_null_password` | Executes `config --username 123456789`; checks the complete JSON value is the synthetic customer ID and null password. Missing username must exit 2 without changing saved settings. |
| `exact_readme_quote_options_reach_real_binary_operation_help` | Passes both exact README quote argument lists to the binary, appending `--help`; expects the appropriate `average`/`last` operation help. It does **not** fetch quotes. |
| `documented_quote_values_validate_before_missing_symbol_without_help` | Runs all eight documented lengths and all five operations with interval 0, deliberately omitting required symbol. Expects only missing-required-argument failure (exit 2), not invalid-value/unknown-operation failure. Negative controls `--length 1M` and `--interval 1` must produce invalid-value errors without `--help`. No accepted quote command reaches dispatch. |
| `quote_help_lists_readme_contract_without_reading_credentials` | Root `--credentials` precedes `quote --help`; a nonexistent synthetic credentials path is not read/created. Help lists all documented options/operations. This checks help, not successful credential-free quote dispatch. |
| `published_guides_do_not_restore_known_unsafe_permission_or_scheduler_examples` | Static fenced-code guard over root/API README, DCA and development guides rejects known `777`/`a+rwx` chmod/chown examples and PowerShell `-NoExit`. This is a narrow regression guard, not an exhaustive semantic security audit. |

**Help precedence caveat:** Clap can process help before required-argument or
value validation. Appending help alone is not evidence that an arbitrary valid
quote would dispatch. The separate no-help positive/negative preflight probes
exercise value validation while ensuring that the required symbol is absent.
The tests intentionally do not use a valid quote invocation as a syntax probe.

Each child has cleared inherited environment, a fresh temporary HOME and explicit
absolute `BOURSO_DATA_DIR`, null stdin, and a temporary working directory. This
also isolates the log file created before parsing. Tests remove their own
synthetic directory on normal completion. Isolation is not a general OS network
sandbox: absence of quote/bank requests relies on the exercised help/config/Clap
preflight paths. Cargo `--offline` prevents registry access, not application HTTP.

## Complementary existing fake/loopback tests

The production parser lives in `src/main.rs`. The successful fake-data dispatch
seam `parse_matches_with_dependencies` is private in `src/lib.rs`; integration
tests cannot call it without changing production visibility. Do not copy a new
Clap parser into this walkthrough as a substitute for the real binary.

- `dispatch_tests::public_quotes_never_load_settings_and_use_only_injected_data`
  invokes the real dispatcher with synthetic ticks and panic-on-use settings/bank
  factories for all five operations. Its matches are constructed by a simplified
  test parser; it does not independently prove README CLI syntax.
- `dispatch_tests::empty_quotes_return_errors_without_panicking` exercises error
  behavior with empty fake ticks.
- `client::http_tests::session_cookies_dynamic_api_and_public_ticks_share_local_transport`
  runs real reqwest against a loopback fixture, checks parsed tick data and the
  request path including `symbol=TEST&length=30&period=0`. Its private transport
  facade disables external fallback/proxies/redirects. This is not a public-bank
  or production-network success claim.

Together, real-parser checks and separately injected dispatcher/loopback checks
provide complementary local evidence; they are not one end-to-end README quote
execution through a fake transport.

## Reproduce with installed Rust 1.96.1, offline

Run from the repository root. Select the **already installed** 1.96.1 toolchain;
never invoke rustup/Cargo after replacing HOME. An explicit toolchain-bin PATH
also avoids wrappers/system Cargo that ignore the repository override:

```sh
export PATH="$HOME/.rustup/toolchains/1.96.1-x86_64-unknown-linux-gnu/bin:$PATH"
# Keep generated artifacts outside the checkout during a two-file-only review.
export CARGO_TARGET_DIR="$(mktemp -d /tmp/bourso-docs-target.XXXXXX)"
rustc --version
cargo --version
cargo test -p bourso-cli --test docs_walkthrough --locked --offline
cargo test -p bourso-cli --lib dispatch_tests::public_quotes_never_load_settings_and_use_only_injected_data --locked --offline
cargo test -p bourso-cli --lib dispatch_tests::empty_quotes_return_errors_without_panicking --locked --offline
cargo test -p bourso_api --lib session_cookies_dynamic_api_and_public_ticks_share_local_transport --locked --offline
rustfmt --edition 2021 --check tests/docs_walkthrough.rs
```

Adjust the installed toolchain path for the host; do not download a compiler or
crates implicitly. These commands require cached dependencies. They do not build
or verify a release distribution artifact. A fresh target directory costs build
time/disk; it can be reused for subsequent checks.

## Observed run: 2026-11-13, Linux

Environment: `rustc 1.96.1 (31fca3adb 2026-06-26)` and
`cargo 1.96.1 (356927216 2026-06-26)`, native Linux, locked offline Cargo,
`CARGO_TARGET_DIR=/tmp/bourso-docs-119-target`. Starting HEAD was
`12099517ebb7914fe28e005f906c7ffe374f4f1b`; concurrent, uncommitted production
changes were present. **Provenance is preliminary/limited, not frozen-candidate
acceptance evidence.** The coordinator must rerun against the integrated frozen
candidate.

| Executed check | Actual result |
| --- | --- |
| `cargo test -p bourso-cli --test docs_walkthrough --locked --offline` | **BLOCKED before tests**, Cargo exit 101: six E0603 visibility errors from concurrent `src/financial_guard.rs` calls into private functions in `src/settings.rs` (`private_directory`, `open_private_file`, `open_existing_regular`, `create_private_file`, `storage_directory`, the latter used twice). None of the five initial integration tests executed in this run. No production correction attempted here. |
| `cargo test -p bourso_api --lib session_cookies_dynamic_api_and_public_ticks_share_local_transport --locked --offline` | **PASS**, exit 0; 1 test passed, 112 filtered out; loopback fixture only. |
| `rustfmt --edition 2021 --check tests/docs_walkthrough.rs` | **PASS**, exit 0. Only this new Rust file was formatted. |
| `git diff --check` | **PASS**, exit 0; this does not check all content of untracked files or prove behavior. |
| Equivalent Python static fenced-code scan over the four guides | **PASS** for the known-unsafe-example patterns described above. This executed source inspection, not the Rust integration test or platform commands. |
| Two dispatcher test commands above | **NOT RUN** in this worker: CLI compilation is blocked; their assertions were inspected in source only. |

No credentials, bank requests, financial commands, release operations, Beads
commands or commits were used. Build artifacts were written outside the checkout.
Only the new test and this document were authored in the repository.

## Remaining local gates and source-only observations

DOCS-1 runtime/parser verification remains open until the CLI compiles and the
six new tests plus existing fake dispatcher tests pass. If required-argument
precedence changes, diagnose the no-help probes rather than treating help success
as a replacement for value validation. A source owner must resolve the unrelated
visibility errors; this walkthrough has no authority to change those files.

DOCS-2/DOCS-3 are not runtime-verified here. Source inspection of the current
README describes null/plaintext password storage, conditional password prompting,
logs, the MFA deadline and lack of an end-to-end timeout. Its DCA section and
`docs/DCA.md` warn against unattended operation and MFA bypass. The narrow static
regression scan passes; it does not independently accept every security promise,
prove terminal behavior, or execute a Windows/macOS scheduler.

The current parallel financial work can invalidate older README/DCA descriptions
of order/transfer behavior. The coordinator should reconcile those descriptions
with the final production candidate before a DOCS-2/DOCS-3 review. No documentation
or code outside the two assigned new files was edited here. No full workspace
suite, Clippy, release build, live quote, bank, DCA launcher, native-platform run
or independent judgment was performed by this worker.

## Integrated correction and replay

The coordinator exposed existing private-storage helpers crate-locally without
changing their behavior, reran the suite, and corrected a help-layout assertion
(`quote --symbol <symbol> average` is valid help). A new discriminating runtime
help test failed on the old misleading “50 characters” text, then the CLI help
was corrected to the unchanged local 50 UTF-8-byte policy with provider contract
unverified. Final acceptance evidence is recorded separately against the frozen
candidate; the worker run above remains preliminary historical evidence.
