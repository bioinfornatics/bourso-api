# Local CI verification and supply-chain inventory

This is the bounded local evidence procedure for `bourso-y3u.37` (LOCALCI-1–3)
and the configuration inventory for `.30` (SUPPLY-1–3). The parent `.29` CI-1
criterion includes real PR/fork triggering; **local checks do not satisfy that
remote criterion**. Canonical criteria remain in those tickets, not this document.

## Run without network or repository mutations

Prerequisites: Python 3.10+ with **already installed PyYAML** (tested with 6.0.3),
Rust/Cargo/Clippy/rustfmt **1.96.1**, cached locked dependencies, and native build
prerequisites. This script does not install dependencies or tools. PyYAML is a
local verifier prerequisite, not a new Cargo/application dependency. A missing
parser stops verification rather than falling back to regex-only YAML checking.

```sh
# Parse the actual YAML/JSON and exercise in-memory negative source controls.
python3 -B scripts/test-ci-local.py --output /tmp/bourso-ci-source

# Additionally run native gates and the deliberate lint failure in a fresh copy.
# Output directories must not already exist. Use your installed compiler path.
python3 -B scripts/test-ci-local.py \
  --output /tmp/bourso-ci-negative \
  --negative-control \
  --toolchain-bin "$HOME/.rustup/toolchains/1.96.1-x86_64-unknown-linux-gnu/bin"
```

Omit `--toolchain-bin` if the correct toolchain is already on PATH. Rustup
installation is disabled; versions are checked before compiling. `--timeout`
sets the per-command timeout (default 600 seconds). Without `--output`, a unique
directory is made under `/tmp`. Exit 0 means **the requested checks** passed;
without `--negative-control`, native execution is explicitly `NOT RUN`.

Outputs include `report.json`, native command/exit records in `commands.json`,
individual logs, and `source-hashes.json`. The verifier copies explicit project
inputs (including untracked source/tests) rather than `git archive`, which would
silently omit the working candidate. It excludes Git/Beads state, existing target
outputs and user home files. No symlinks are accepted in copied inputs. The copy,
build artifacts, temporary HOME/config and logs stay under `/tmp`. No source is
injected into the checkout. Source hashes are checked before/after copying and
execution; concurrent input changes invalidate the result. These hashes are
limited local provenance, **not the coordinator's full frozen candidate manifest**.

Native baseline commands are equivalents of the declared gates, with offline
resolution added and the pinned compiler selected by PATH/environment:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --locked --offline
bash scripts/package-release-test.sh
```

Only after all four clean baseline commands succeed does the verifier create
`tests/ci_local_negative_control.rs` **in the copy**. Its `value + 0`
expression triggers the default-warning lint `clippy::identity_op`. First a run
without `-D warnings` must succeed with that exact warning. Then the CI Clippy
command with `-D warnings` must fail. Both use JSON diagnostics; passing requires Cargo exit 101 and error diagnostics
specifically for `clippy::identity_op` in that injected file. A missing tool, offline
cache miss, disk quota error, visibility/compiler error, unrelated lint or clean
baseline failure is **not** a passing negative control. The injected file is
removed after success; failed copies/logs are retained for diagnosis.

The child environment is allowlisted and has a temporary HOME/config, no bank
or GitHub secret environment variables, and `CARGO_NET_OFFLINE=true`. Existing
Cargo/Rustup homes provide cached packages/tools. This is not an OS network
sandbox: trusted local tests/build scripts can make system calls. The verifier
never invokes a bank, GitHub, release uploader, registry fetch or remote workflow;
review changed test/build code before running it. No real credentials are needed.

## Source contracts versus execution

The parser preserves the Actions `on` key instead of interpreting it as a YAML
1.1 boolean, rejects duplicate keys, and checks the actual workflow data structure.
The intentionally strict policy fails closed on unreviewed workflow/action/image
changes. Updating pins requires reviewing both configuration and inventory.

Source checks cover:

- Unfiltered `pull_request` gates, main/tag pushes, all three OS matrix entries,
  exact fmt/workspace Clippy/workspace test/packaging commands, and no conditional
  gate steps or `continue-on-error`.
- All ordinary actions against the reviewed full commit SHA inventory below;
  only the exact SLSA reusable-workflow exception is permitted.
- Checkout `persist-credentials: false`; gate/build/coverage checkout references
  exactly `${{ github.sha }}`. Local reusable coverage uses the caller's revision.
- Exact job `needs` and condition contracts, read-only compilation permissions,
  no inherited secrets, publisher without source checkout/execution, and
  provenance bound to `needs.subjects.outputs.hashes`.
- The devcontainer image digest and absence of added privileges, seccomp
  overrides, capabilities, mounts, environment credential surfaces or features.

Thirteen unsafe **in-memory** mutations test the checker: tolerated/masked lint,
skipped gate, `always()` publisher, removed gate edge, wrong checkout SHA,
floating action, write-enabled build, persisted checkout credentials, a bank
secret, inherited reusable-workflow secrets, floating image and unconfined seccomp.
They test rejection by the local checker, **not GitHub's scheduler**.

Release source DAG:

```text
gates (Linux/macOS/Windows) ─┐
coverage (same-revision) ────┴─> build (tag only) -> subjects
              gates + coverage + subjects ──> publish (tag only, draft)
                                  subjects + publish ──> provenance
```

No `always()`/`continue-on-error` bypass is declared. Given GitHub's normal
`needs`/success semantics, a failed gate prevents the downstream release path.
That is a **source-derived inference**, not an observed remote failure/publication
attempt. Source checks also do not prove that an immutable upstream action is
benign, that an artifact is reproducible, or that a remote run used this file.

## Reviewed action/image inventory

References below are read from `.github/workflows/{cicd,coverage,scorecard}.yml`
and `.devcontainer/devcontainer.json`. Version labels are explanatory source
comments, not registry lookups or remotely verified provenance.

| Action | Full pinned commit | Use |
|---|---|---|
| `actions/checkout` (v4 label) | `11d5960a326750d5838078e36cf38b85af677262` | CI gates/build; coverage |
| `actions/checkout` (v3.1.0 label) | `93ea575cb5d8a053eaa0ac8fa3b40d7e05a33cc8` | Scorecard |
| `actions/upload-artifact` (v4) | `ea165f8d65b6e75b540449e92b4886f43607fa02` | Release archives/checksums; coverage; SARIF |
| `actions/download-artifact` (v4) | `d3f86a106a0bac45b974a628896c90dbdf5c8093` | Subjects/publisher |
| `softprops/action-gh-release` (v2.0.4 label) | `9d7c94cfd0a1f3ed45544c887983e9fa900f0564` | Draft publication |
| `codecov/codecov-action` (v4.3.0 label) | `84508663e988701840491b86de86b666e8a86bed` | Optional coverage upload |
| `ossf/scorecard-action` (v2.3.1 label) | `0864cf19026789058feabb7e87baa5f140aac736` | Scorecard analysis |
| `github/codeql-action/upload-sarif` (v2.2.4 label) | `17573ee1cc1b9d061760f3a006fc4aac4f944fd5` | Code-scanning upload |

Devcontainer image:

```text
mcr.microsoft.com/devcontainers/rust:1-bookworm@sha256:86d9228ec0744191b45ec91e2bb9a0dd7d7bfd4c4f01ef47abb78032ddddf1c7
```

No coverage container remains: coverage uses native Rust/LLVM instrumentation on
the hosted runner, not a privileged tarpaulin image. The devcontainer only adds
`--name bourso-api-dev`, uses `vscode`, and declares no privileged mode, added
capability or disabled seccomp. Runtime launch/kernel enforcement was **not
observed**. VS Code extensions, hosted-runner images, rustup-distributed toolchain
components and transitive action dependencies are not made immutable merely by
these top-level pins. `ubuntu-latest` remains in coverage/Scorecard; other runner
labels also denote evolving images, not image digests.

### Narrow SLSA exception

```text
slsa-framework/slsa-github-generator/.github/workflows/generator_generic_slsa3.yml@v2.1.0
```

The workflow documents that SLSA verifier identity validation requires a version
tag for this generator rather than a commit SHA. The only allowed exception is
this exact path/tag in the provenance job. The configuration cites the
[generator's versioned documentation](https://github.com/slsa-framework/slsa-github-generator/blob/v2.1.0/internal/builders/generic/README.md).
That external requirement and tag resolution were **not refreshed over the
network** by this local verification. A semver tag is not immutable like a full
commit hash: retain the trust exception and independently verify generated
provenance against final archives before any release acceptance.

## Permissions and secret scopes

| Workflow/job | Declared token scope | Secret exposure / boundary |
|---|---|---|
| CI gates, build, subjects, reusable coverage | `contents: read` | No bank secrets; no `secrets: inherit`; checkout credentials not persisted |
| CI publish | `contents: write` | Artifact-only draft publication; no checkout/compilation; implicitly available GitHub token is still a credential |
| CI provenance | `actions: read`, `id-token: write`, `contents: write` | OIDC signing and draft provenance attachment, after publication; no compilation token inheritance |
| Coverage test | `contents: read` | `CODECOV_TOKEN` appears only as input to the push-only Codecov step; fork PR steps receive no explicit secret reference |
| Scorecard analysis | `security-events: write`, `id-token: write` | Separate code-scanning/result publication; not a build/test job. No active PAT input; commented `SCORECARD_TOKEN` is not executable configuration |

CI/coverage defaults are `contents: read`; Scorecard's workflow default is
`read-all`, overridden by the analysis job mapping. Unspecified permissions in
explicit job maps are none under GitHub's permission semantics. Codecov upload
failure is deliberately nonfatal (`fail_ci_if_error: false`); compilation/tests,
coverage generation and required artifact reporting remain separate fatal steps.
No claim is made that *all* jobs are secret-free or write-free.

## Remaining validation

- Execute the optional native path on a stable candidate with sufficient local
  disk quota and complete cached dependencies. Always inspect `report.json`:
  a source-only PASS is not a native PASS.
- Re-run instrumented API coverage separately with `scripts/coverage.sh --offline`
  in an isolated copy when needed; this verifier checks its wiring, not its
  instrumentation/report correctness or a coverage percentage threshold.
- Real PR/fork triggering, GitHub job token behavior, tag publication suppression,
  macOS/Windows native execution and hosted runner availability remain remote
  CI obligations. Linux fixture packaging checks Windows archive structure, not
  execution of a Windows binary.
- Branch protection, required status checks, release permissions, environments,
  organization policy and protections against workflow-file changes are external
  and **NOT VERIFIED**. This DAG cannot itself prevent merging an unprotected PR.
- No remote CI, publishing, image pull/launch, live bank request or credentialed
  operation is part of this procedure. Independent review of a frozen candidate
  and raw evidence is still required; neither this document nor a local PASS
  closes the parent tasks or approves a release.
