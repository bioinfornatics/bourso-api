# Known limitations and unresolved contracts

This is a source-grounded maintenance snapshot, not a production acceptance
report or a substitute for Beads. The
[13 November 2026 review](reviews/2026-11-13-code-review.md) records earlier
findings; some have since received local fixes. Passing synthetic tests establishes
only their exercised scenarios, not live bank compatibility or absence of loss.

## Financial confirmation remains unsafe

- **Orders:** explicit side/nonzero quantity and calendar-valid expiration are guarded locally. Preparation/check messages require review, and check echoes must match side, quantity, limit, type, expiration and any supplied resource ID before confirmation. The detailed API returns `SubmittedUnverified` and explicitly accessible provider state, not a trusted execution outcome. The legacy tuple remains available. An order ID is not execution proof.
- **Prices and semantics:** order amount versus unit-price/total semantics and
  provider validity rules need evidence. Existing `f64` order fields are not an
  exact-money guarantee. Local date validation establishes a `YYYY-MM-DD`
  calendar date, not market timezone/current-date/maximum-validity policy.
- **Transfers:** final HTTP 200 now produces `SubmittedUnverified`; the word `Confirmation` no longer establishes success. The existing submission stages remain available. Intermediate transition meanings, trusted receipt and positive provider execution semantics still need evidence.
- **Unknown outcomes:** public typed `OutcomeUnknown` covers final submission send/read/status failures and order parse failures, with redacted diagnostics and a sensitive correlation-reference accessor. This error type is not durable journaling or a complete reconciliation API. Raw library re-invocation and dropped library futures remain unguarded. An additive opt-in [library replay barrier](LIBRARY_REPLAY_GUARD.md) now reserves before invocation with an explicitly supplied local Unix directory, retaining the journal after every outcome/drop/crash. Only cooperating callers using the same domain are protected; no power-loss, provider idempotency or global recovery guarantee is made.
  The CLI now has a conservative private replay barrier before financial API
  invocation, with explicit results and operator review; it deliberately retains
  possibly-not-emitted attempts. See [FINANCIAL_GUARD.md](FINANCIAL_GUARD.md). Do not replay before independently checking the bank.
- **Cancellation:** a library method exists but its provider contract is not
  established, and the CLI does not expose a cancellation command.

The blocking evidence work is `bourso-y3u.4` (provider financial contracts).
Unknown-outcome handling is `bourso-y3u.8`. Fixes must be grounded in reviewed,
legally obtained anonymized fixtures, not invented status semantics. Until then,
financial commands are **not recommended for production use**. No dry-run or
mandatory final human confirmation is promised by their CLI syntax.

## DCA automation: DO NOT USE

`bourso-launchd.sh` now journals attempts privately, observes its foreground child and locks concurrent runs. It never equates child exit zero with verified financial success. Unknown attempts require operator reconciliation and block re-emission. See [DCA.md](DCA.md). This is local safety, not unattended financial automation.

launchd `RunAtLoad` can trigger an additional immediate attempt besides the
calendar schedule. Windows/cron scheduling does not remove password, TTY or MFA
requirements. The workflow is **semi-interactive**, not safely unattended.

**Do not use unattended DCA automation while trusted bank outcomes and contracts
remain unverified (`bourso-y3u.4`, `.6`, `.7`, `.8`).** No scheduler installation recipe is endorsed. Removing unsafe
Windows/macOS examples from the README does not establish trusted financial confirmation or satisfy
its success/locking/recovery criteria. Local documentation and fake-executor safety can be verified independently;
neither establishes automation readiness. DCA cancellation forwards TERM to the
direct child and retains its lock conservatively; a live child may remain. The
CLI and DCA have separate journals requiring separate operator review.

## CLI and data contracts

- `config` requires `--username`; it overwrites managed settings with a null
  password. JSON files may nevertheless hold plaintext passwords. The current
  unconditional password-prompt message does not match that conditional behavior.
- `quote --length` accepts numeric enumerated values only, not `1M`. Its help
  hint is stale. `last` retains the requested period, rather than overriding it
  to one day as its help claims. It prefers usable `qd` then the final history
  entry, without searching farther back. Public quotes still require network
  access and are not promised to be real-time.
- Highest/lowest/average now return `Result<f64>`; downstream callers need to
  handle errors rather than expect an infallible scalar. No package version
  alone should be used as evidence of source compatibility.
- Transfer input uses exact integer cents and rejects excess precision. The
  representable maximum is not a bank allowance. Balances and transactions still
  expose `f64` compatibility fields after checked bank-cent parsing.
- Transfer reasons retain the existing **50 UTF-8 byte local policy; provider
  contract unverified**. The API documentation/error now labels that policy;
  the CLI help labels the same byte policy and unverified contract. Source
  inspection established `String::len()` versus the old “50 characters” text;
  local runtime tests independently establish the unchanged accepted units:
  50 ASCII bytes pass, 51 fail; 25 `é` (50 bytes) pass, 26 fail; 12 `😀`
  (48 bytes) pass, 13 fail; 16 decomposed `e` + U+0301 units (48 bytes) pass,
  those 16 units plus `ab` (50 bytes) pass, 17 units (51 bytes) fail.
  Empty/default reasons remain accepted, without normalization or trimming.
  Both API entrypoints reject over-limit reasons before any transfer request;
  accepted cases reach local initialization only, not verified bank acceptance.
  The regression's pre-change failure concerns the misleading error text, not
  the byte boundary: no change to accepted inputs was necessary or intended.
  These local tests and source evidence do not establish a provider length unit
  or allowance. `bourso-y3u.26` is partial: **REASON3 remains unresolved and
  cannot be closed** without justified provider evidence (`bourso-y3u.4`).
- Transaction export only accepts the verified eleven-column UTF-8 CSV schema;
  variants or unexpected pages fail rather than produce a partial/empty success.
  Dates are passed in `DD/MM/YYYY` form; do not infer complete date-range/provider
  validation from the CLI help. HTML/layout changes can break scraping.
- CSV quoting is not formula neutralization. Import untrusted fields as text.
  Output files are sensitive. Stdout may mix CLI presentation with data; prefer
  `--output` rather than treating stdout as a pure JSON/CSV protocol.

## Platform, transport and evidence limits

Unix private-file behavior does not establish Windows ACL safety. Export rename
is not a guarantee of parent-directory durability across crashes or protection
against malicious same-user filesystem races. Logs/settings/exports require
trusted directories and deliberate retention.

The CLI's 300-second deadline applies to MFA polling, not every HTTP request or
whole command. The production client has no universal request-timeout policy or
outbound-origin allowlist. The specific export redirect guard does not cover all
configuration-derived URLs.

Rust 1.96.1 is the supported/tested baseline; lower MSRV compatibility is not
established. API loopback tests exercise shared code through a private transport
facade; they do not attest to production transport configuration, real MFA,
market/execution behavior or bank HTML compatibility. The coverage script reports
**API-only** coverage, including test/support modules, with no percentage gate;
it is not CLI coverage or proof of complete behavioral coverage.

No fork binary publication, provenance verification, platform matrix or live-bank
acceptance is asserted by these documentation changes. Upstream URLs remaining
in Cargo metadata/startup output do not redirect users to a verified fork build.
See [development](DEVELOPMENT.md) for bounded local checks and
[security](SECURITY.md) for operational precautions.
