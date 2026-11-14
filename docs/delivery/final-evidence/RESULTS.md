# Verified local delivery — 2026-11-13

Beads is the live tracker. This report records evidence, not a second roadmap.

## Exact candidate and commits

Acceptance candidate: `7d8e59caafb3402c38ab73d9a8df4121df4cb90bf95294b8c9d37262dd78c204`.
Inputs: `../final-verification/scope.json`, canonical ticket snapshot `criteria.json`.
All relevant code/tests/fixtures/scripts/configuration/docs/legal texts are included;
`final-evidence` and historical `evidence-96` are output-only exclusions. No report
under an output exclusion was consumed by the verification commands. Reviewers
read raw outputs subsequently as evidence, not as build/test inputs.

`before.json` / `after.json`: SAME. Independent reviewers' captures also SAME.
After source commits `237fc5f`, `7edfd2e`, `9fd834d`, the new capture
`after-source-commits.json` differs only in Git HEAD/index and candidate ID;
entries (bytes/modes), root, projection and scope/environment are identical.
This is explicit content/environment equality, not a claim that precommit checks
ran on the new commit identity. Proofs retain their original candidate ID.

## Gates executed on the frozen working tree

| Check | Result | Evidence |
| --- | --- | --- |
| Rust/Cargo | 1.96.1 explicit installed toolchain PATH | scope environment, CI version logs |
| `cargo fmt --all -- --check` | PASS | fmt.log |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | PASS | clippy.log |
| `cargo test --workspace --locked --offline` | PASS, 194 tests: 81 CLI + 113 API | tests.log |
| `bash scripts/test-dca.sh` | PASS, fake executors only | dca.log |
| `bash scripts/package-release-test.sh` | PASS, fixtures | package.log |
| legal fixture replay | PASS: 3 archive containers, 313 exact upstream texts each, 181 third-party packages | legal-run/inspection.json, commands.json, archives/hashes |
| `scripts/coverage.sh --offline` | PASS, 113 instrumented API tests | coverage.log, coverage/ |
| CI source + native negative control | PASS, clean baseline; injected identity_op warning exit0 then strict error exit101 | ci/report.json, commands.json, raw JSON diagnostic logs |

Two intentionally ignored harness entries are child-process helpers executed
explicitly by parent tests, not missing acceptance scenarios. Coverage is
API-only including test/support sources: regions **96.22%**, functions **93.76%**,
lines **97.31%**. No threshold invented; no CLI/dependency or bank compatibility
coverage claim. Production API method bodies compile normally and instrumented;
private test transport is loopback-only. All reports retain that limitation.

Observed new red evidence: CLI missing-credentials result (worker117), old
50-character help (`/tmp/bourso-help-red.log`), DCA supervisor deferred signal
(worker121). These are actual preliminary reproductions; no historical red is
claimed for previously implemented order/transfer guards or formatting.

## Independent acceptance and tracker outcome

Review receipts in REVIEWS.md; raw reruns in reviewer-125/ and reviewer-126/.
Both independent reviewers accepted their bounded local scope without changing
candidate. Small-task slice-acceptance v1.1.0, numerical score N/A.

Closed and read back: `.27`, `.28`, `.30`, `.31`, `.32`, `.35`, `.36`, `.37`, `.38`.
The five new slices keep previously mixed local work traceable. Original criteria
were not deleted or rewritten. Nine local deliveries closed in this run; total
**29 closed children / 38 children**, **9 remaining**, plus the epic.

Dependency removals and reasons are in native Beads notes. `.34` gained explicit
mandatory prerequisites before excessive transitive blockers were removed. Parent
links preserved; `beads-cycles.txt` confirms no cycle. Snapshot final lists are
outputs only; live `bd list --all` and `bd ready` remain authoritative.

## Concrete remaining obligations

- `.4`: CONTRACTS-2, provenance/anonymization of existing supplier financial
  examples absent. Local matrix `.38` delivered. Evidence custodian to designate.
- `.6`: contract-valid check/positive state mapping (ORDERCHECK-1/2); local
  divergence/messages block confirm and unverified results remain available.
- `.7`: expected intermediate forms/transitions (TRANSFERPROOF-2), trusted receipt
  with source/destination/amount/reference (TRANSFERPROOF-3). Bodies currently
  not contract-validated; no speculative success parser or feature disablement.
- `.8`: direct library reprise/reconciliation scope decision (UNKNOWN-2) and
  supplier certain success/refusal semantics (UNKNOWN-3). Typed post-POST unknown,
  no automatic application retry, CLI/DCA local replay barriers already delivered.
- `.24`: cited market timezone/status/calendar, including DST/holidays/midnight.
- `.26`: provider-backed reason unit/limit; unchanged honest local 50 UTF-8 bytes.
- `.29`: actual remote PR/fork CI run and native platforms; configuration/local
  negative control `.37` accepted. Branch protection remains explicitly unverified.
- `.33`: authenticated isolated historical FORMAT-2 diff/before-tree absent;
  historical FORMAT-3 comparison not proved. Current fmt/test `.35` accepted;
  no reconstructed history. Obtain authentic evidence or explicit risk/rebaseline
  decision, preserving the gap.
- `.34` and epic: global mandatory criteria not all proved; not closed.

Next actionable step: nominate the custodian and provide an existing legally
obtained anonymized packet listed in `docs/FINANCIAL_CONTRACTS.md`, or resolve
the direct-library recovery/format-history decisions explicitly. No new bank
operation, push, tag, publishing, deployment or remote sync was performed.

## Preserved capabilities and limits

Order and transfer submission APIs and compatibility entrypoints remain enabled.
CLI uses order_detailed, displays fixed redacted results independent of logs and
retains unresolved local attempts. Exit2 = this invocation did not invoke its
financial API; exit3 = response-unverified; exit4 = outcome-unknown, possibly not
emitted. None asserts bank acceptance/execution. Operator review is explicit and
not a receipt authenticator. API consumers remain outside the CLI guard.

DCA observes the actual child, retains unknown/unverified state, blocks repeats,
and keeps its lock on cancellation even if a child survives. CLI and DCA journals
are separate and require separate review. No fsync/power-loss/global exactly-once,
Windows ACL, native macOS/Windows, signed provenance or production readiness proof.
