# Development

## Scope and prerequisites

This repository contains the `bourso-cli` executable and the `bourso_api` library
in `src/bourso_api`. The workspace selects both by default and shares **one root
Cargo.lock**, including when Cargo is invoked from the API directory. Use
`--locked`; do not regenerate a nested lockfile.

Both manifests declare Rust **1.96.1** as the supported minimum (MSRV), and
`rust-toolchain.toml` pins it with rustfmt and Clippy. Lower-version support has
not been verified. A newer compiler passing is not MSRV evidence. Check the
actual compiler and Cargo on PATH, especially with wrappers or system packages:

```sh
rustc --version
cargo --version
rustc -vV
```

With an already installed rustup toolchain, use `cargo +1.96.1 ...`, or place that
toolchain's `bin` directory first on PATH. Plain system Cargo may not honor the
rustup override. Missing compilers/components require a separate installation;
the coverage script never installs them automatically.

Build prerequisites:

- Cargo/Rust 1.96.1 and a platform native linker/C toolchain.
- The default reqwest TLS backend's native dependencies: typically OpenSSL
  development headers/libraries and pkg-config on Linux. macOS/Windows use their
  platform TLS integration; cross-compilation needs appropriate target tooling.
- Cached crates for offline mode, otherwise access to the configured Cargo
  registry. Cargo downloads are not bank traffic.
- Bash, Python 3 and compatible LLVM tools for the coverage script.

The lockfile fixes dependency resolution, not the operating system/toolchain or
bit-for-bit build reproducibility. Consult the manifests for dependency features
and the root lockfile for resolved versions.

## Build and test from the repository root

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo doc -p bourso_api --no-deps --locked
```

Add `--offline` to Cargo build/test/clippy/doc commands only when dependencies are
already cached. For example:

```sh
cargo test --workspace --locked --offline
cargo test -p bourso_api --locked --offline
```

Build artifacts go under `target/debug/` or `target/release/`, and Rustdoc under
`target/doc/`. Test totals evolve: record the command, compiler, source candidate,
exit status and observed results rather than relying on a fixed README count.
A suggested check is not a claim that it currently passes.

Tests use synthetic fixtures and local data, not bank accounts. API HTTP unit
tests use real reqwest requests against a bound loopback server. Their private
`cfg(test)` facade rewrites destinations to loopback, disables proxies and
redirects and has no external fallback. The normal public method bodies are
shared, but production transport configuration is not fully exercised. CLI tests
use isolated HOME directories or injected dependencies. Never use real
credentials/financial submissions as a test fixture.

## Safe documentation/CLI checks

The binary initializes logging **before argument parsing**. Even `--help` and
invalid commands may create `~/.bourso` and acquire its logger lock. For local
help/config checks use a fresh temporary HOME, a built executable and synthetic
data; avoid invoking Cargo/rustup after changing HOME:

```sh
binary="$PWD/target/debug/bourso-cli"
check_home="$(mktemp -d)"
HOME="$check_home" XDG_CONFIG_HOME="$check_home" "$binary" --help
HOME="$check_home" XDG_CONFIG_HOME="$check_home" "$binary" config --username 123456789
HOME="$check_home" XDG_CONFIG_HOME="$check_home" "$binary" quote --help
HOME="$check_home" XDG_CONFIG_HOME="$check_home" "$binary" trade order new --help
HOME="$check_home" XDG_CONFIG_HOME="$check_home" "$binary" transfer --help
# Inspect/remove this synthetic temporary directory when finished.
```

Do not run valid quote commands to check syntax: they contact the public market
service. Do not execute valid trade/transfer inputs as a parser probe. Use help,
parser tests or injected fake dependencies. A help command does not prove every
option's runtime semantics: known quote help discrepancies are documented in the
[root README](../README.md).

## API-only coverage

Run with Rust 1.96.1 on PATH:

```sh
bash scripts/coverage.sh --offline
# Or, when registry access is intended:
bash scripts/coverage.sh
```

The script requires `llvm-profdata` and `llvm-cov` from the compiler's
`llvm-tools-preview` component or compatible system LLVM tools. Their LLVM major
must match `rustc -vV`; actual profile merge/report generation must succeed.
Only zero arguments or `--offline` is accepted. Nothing is installed by the
script. Missing tools/version mismatch is a failed prerequisite, not zero coverage.

It compiles the ordinary non-test API and instruments
`cargo test -p bourso_api --all-targets`, using LLVM source coverage, without the
`tarpaulin_include` cfg. There is **no percentage gate** and no list of omitted
API functions. Do not use cfg exclusions to improve a score.

Artifacts are emitted under `target/coverage/run.*/` (build cache under
`target/coverage/build/`):

- `summary.txt`, `functions.txt`, `lcov.info`, `html/`;
- `toolchain.txt`, `tests.txt`, `SCOPE.txt`;
- executable manifest and raw/merged profiles for that run.

**Scope:** the API package's Rust sources, including test/support modules.
Dependencies and **the CLI are outside this report**. LLVM mapping and generated
code behavior govern visibility; unexecuted functions remain visible where
mappings exist. Coverage is not equivalent to behavioral acceptance, complete
branch/path testing, production transport coverage or live bank compatibility.
The coverage workflow invokes this script; an artifact/upload is not proof of a
successful published release.

## Contributions and review

Read the current source alongside [the historical review](reviews/2026-11-13-code-review.md):
its findings describe an earlier candidate and are not an up-to-date test report.
Beads owns roadmap/dependency state; these documents do not replace it. Preserve
other contributors' changes, add discriminating local tests and review errors as
well as happy paths. Financial provider assumptions need legally obtained,
reviewed anonymized evidence, not guessed success predicates.

Read [SECURITY.md](SECURITY.md) and [KNOWN_LIMITATIONS.md](KNOWN_LIMITATIONS.md).
Do not collect bank captures, call the bank, release or deploy as part of ordinary
checks. The DCA launcher is not a development smoke test.
