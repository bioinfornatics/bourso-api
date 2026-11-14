# Financial contracts: local matrix, not a bank specification

Updated 2026-11-13 from current source and the H01 review. Canonical remaining
criteria are in Beads `.4`, `.6`, `.7`, `.8`, `.24`, `.26`; local matrix `.38`.
No provider capture, documentation establishing these meanings, or authenticated
bank receipt was found in the inspected repository/history. Existing loopback
fixtures are deliberately synthetic. They are not anonymized bank evidence.
CONTRACTS-2 remains unverified: no source financial example has been supplied to
check provenance/anonymization. No real banking operation was performed.

## Current source-grounded matrix

| Field/stage | Established local behavior | Provider meaning still unknown |
| --- | --- | --- |
| Order side/quantity | Explicit requested side/nonzero quantity takes precedence; check echo must match | Lots/fractions, provider normalization and eligibility |
| `orderAmount`, `orderPriceLimit` | Existing f64 options retained; limit conversion is not an exact-money guarantee | Total versus unit amount, currency, fees, tick and rounding; quantity >1 needed |
| Order validity | Calendar-valid YYYY-MM-DD preserved and compared | Market timezone/current-date/maximum validity policy |
| Prepare/check messages | Any nonempty acceptability messages require review, not an invented allowlist | Nominal/warning/refusal states, blocking messages and acknowledgment rules |
| Check echo | Side, quantity, type, limit, expiration, supplied resource ID compared before confirm | Additional economic fields, normalization and supported order types |
| `resourceId` | Preparation/check/confirmation correlation used locally | Scope, rotation, lifetime and idempotency; not a safe retry key |
| Confirm `orderId`, `ordStat`, label/message | Detailed outcome is SubmittedUnverified, provider fields explicitly sensitive | Trusted deposited/accepted/rejected/executed state mapping; an ID is not execution proof |
| Cancellation | Existing library POST remains available; no CLI cancel | Reference linkage, HTTP-200 refusal, accepted cancellation and final state |
| Transfer initialization/instance | Existing Location/form instance extracted | Flow identifier format/lifetime and instance semantics; not automatically CSRF |
| Transfer steps /2,/3,/4,/5,/8,/10 | Existing stages and multipart fields preserved; exact integer-cent input; minimum 10 EUR local policy | Allowed transitions, duplicated stages, skipped suffixes, eligibility, schedulingType and currency/bounds |
| Intermediate transfer responses | HTTP status checked, bodies not contract-validated | Expected next forms, login/error recognition, source/destination/amount echoes; TRANSFERPROOF-2 still open |
| Transfer final HTTP 200 | Body received -> SubmittedUnverified, no success from the word Confirmation | Trusted receipt/reference and acceptance/execution, refusal distinction |
| Transfer reason | Local maximum 50 UTF-8 bytes, empty/default accepted; ASCII/accent/emoji/combined boundaries tested | Provider maximum and octet/scalar/grapheme unit; no speculative chars() change |
| Market opening | Conservative Unknown when reliable data unavailable, not inferred from local wall clock | Market identifier, timezone, trading status/calendar including DST/holidays/overnight |
| Final POST send/read/status/parse errors | Typed OutcomeUnknown and redacted correlation; no application retry loop | Whether operation applied and trusted reconciliation endpoint/receipt |
| CLI/DCA attempts | Private local guards retain unresolved attempts before invocation, explicit operator review | Not global idempotency, automatic reconciliation or trusted execution |

Sources: `src/bourso_api/src/client/trade/order.rs`, `order_tests.rs`,
`client/transfer/{mod.rs,money.rs,submission_tests.rs}`, `client/outcome.rs`,
`client/trade/tick.rs`, `src/financial_guard.rs`, `bourso-launchd.sh`.
Synthetic tests exercise guarded control flow, not the unknown supplier contract.

## Minimum outstanding evidence and next action

The user/maintainer holding **existing legally obtained evidence** must nominate
an evidence custodian (currently unassigned). Do not create new bank transactions
for these tickets under the current authorization. Supply only reviewed,
anonymized material, with date/site version, scenario, provenance and independently
observed final state. Replace IDs consistently to preserve equality/correlation;
remove accounts, personal names, cookies, Authorization, credentials and tokens.
Do not submit a raw HAR or active secrets.

| Ticket / exact outstanding criterion | Minimum evidence | Already delivered / can continue |
| --- | --- | --- |
| `.4` CONTRACTS-2 | Existing financial examples plus provenance and anonymization review | Local matrix and explicit unknowns; local mocks/docs/packaging do not wait |
| `.6` ORDERCHECK-1/2, contract portion | Prepare/check nominal, warning, refusal; quantity >1 amount/limit/fee examples; correlated confirm with attested outcome | Divergence/messages stop confirm; matching synthetic intention still submits; CLI no execution claim |
| `.7` TRANSFERPROOF-2/3 | Complete allowed transitions including login/error/unexpected forms and trusted receipt carrying source/destination/amount/reference | Submission stages preserved, exact cents and unverified final response; cannot invent positive transition parser |
| `.8` UNKNOWN-2/3 beyond local guards | Trusted positive/refusal/reconciliation semantics and remaining original scope acceptance | Typed unknown errors, CLI/DCA guards and additive opt-in library guard `.42` ([scope](LIBRARY_REPLAY_GUARD.md)); raw/direct API consumers remain responsible for safe orchestration |
| `.24` MARKET-1/2 | Cited market timezone/status/calendar; DST, holidays, midnight examples | Conservative Unknown without trustworthy data |
| `.26` REASON-1/3 | Provider policy or validated existing Unicode examples identifying unit and limit | Honest unchanged 50-byte local policy and pre-request boundary rejection |

Positive supplier acceptance remains open, not silently replaced by an
unknown-only contract. API compatibility methods remain available. The independent
global review `.34` and epic cannot close while their mandatory obligations are
unproved. CI remote runs and historical FORMAT-2 are separate non-bank evidence
gaps, not prerequisites for the local matrix.
