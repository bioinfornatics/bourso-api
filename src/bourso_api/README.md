# Bourso API — unofficial Rust library

Library used by the [Bourso CLI maintenance fork](../../README.md).
This is not an official BoursoBank SDK. Live bank compatibility and safe
financial confirmation are not established; read the
[known limitations](../../docs/KNOWN_LIMITATIONS.md) before using financial APIs.

## Workspace and dependencies

Package `bourso_api` is a member of the root Cargo workspace, with Rust **1.96.1**
as its supported minimum and a shared root `Cargo.lock`. Run from the root:

```sh
cargo build -p bourso_api --locked
cargo test -p bourso_api --locked
cargo doc -p bourso_api --no-deps --locked
```

The library documentation is generated under `target/doc/bourso_api/`.
See [DEVELOPMENT.md](../../docs/DEVELOPMENT.md) for native TLS prerequisites,
offline checks and API-only coverage. Reqwest provides HTTP/cookies; serde and
CSV parse responses; Tokio drives asynchronous operations. No live credentials
are required for the local test suite.

## Public API and compatibility changes

`bourso_api::get_client()` and `BoursoWebClient::new()` still return a client
directly, **not** `Result`. Construction alone does not log in. Authenticated
calls require successful `init_session()` and `login()` and may require MFA.
Do not print/debug the client, configuration, cookies or authentication data.
The library does not install the CLI's filtered logger for consumers.

The current source includes fallible API changes: code written against older
signatures must be updated, even though the package version alone may not show
the difference.

- `D::get_highest_value`, `get_lowest_value`, and `get_average_value` now return
  `anyhow::Result<f64>`, not `f64`. Handle `Err` or propagate with `?`; do not
  substitute zero for malformed/empty data. They reject non-finite OHLC values;
  average also rejects sum overflow.
- `D::get_latest_quote() -> Option<QuoteTab>` prefers usable `qd`, then the final
  history entry. Handle `None`; it does not scan backward through history.
  Existing `get_last_quote`/`get_second_to_last_quote` accessors remain available.
  `get_volume()` retains its existing sum contract, including zero for empty data.
- Authentication/config/virtual-pad parsing is fallible. `Config::user_hash` is
  optional before login; authenticated operations require a nonempty value.
  Failed initialization must not be treated as a usable login session.
- Account retrieval returns errors instead of silently discarding failed
  categories. Transactions require the known eleven-column schema and valid
  amounts; unexpected pages are not empty exports.

For example, consuming an already retrieved quote response (no HTTP here):

```rust
use bourso_api::client::trade::tick::GetTicksEOD;

fn average_close(quotes: &GetTicksEOD) -> anyhow::Result<f64> {
    quotes.d.get_average_value()
}
```

### Exact transfer input and mandatory session

Use `client::transfer::money::Money` and `FinancialSession::transfer_money`:

```rust
use bourso_api::client::transfer::money::{Money, MoneyError};

fn exact_amount() -> Result<Money, MoneyError> {
    let amount: Money = "10,50".parse()?;
    assert_eq!(amount.cents(), 1050);
    assert_eq!(amount.bank_amount(), "10,50");
    Ok(amount)
}
```

Input is unsigned ASCII decimal euros, with zero to two fractional digits
(dot/comma), at least 10 EUR, and at most `u64::MAX` cents. Signs, whitespace,
exponents, grouping and excess precision are rejected, never rounded.
No public float/raw compatibility entry point remains. Financial submissions
require an explicit `FinancialSession::open(absolute_trusted_directory)`.
`session.transfer_money(&client, amount, from, to, reason).await` consumes the full
internal stream and returns `Result<anyhow::Result<TransferProgress>, GuardError>`;
only `SubmittedUnverified` is a terminal response, never verified execution.

Balances and transaction amounts still expose `f64` for compatibility. Bank
strings are parsed through checked exact cents, but that does not turn all
public monetary fields or order calculations into decimal types.

### Exports and financial calls

`transactions_to_csv` quotes fields, preserving their literal contents. It does
not neutralize spreadsheet formulas; import untrusted CSV as text, with formula
evaluation disabled. `write_export_atomically` publishes fully prepared content
through a synced sibling temporary file; it does not itself validate arbitrary
caller-provided bytes. Protect the destination directory and exported data.

`FinancialSession::order` preserves explicit side/nonzero quantity and validates calendar dates,
but check/confirm business semantics remain unsafe/unverified. Its returned
`OrderSubmission` is **not** proof of execution. Transfer confirmation
likewise lacks a verified bank receipt contract. Public typed `OutcomeUnknown`
identifies final submission failures, with expurgated diagnostics and an explicitly
sensitive reference accessor; it is not durable recovery or reconciliation.
Never retry a financial POST blindly after an error or a dropped future.

`FinancialSession` reserves the `.42` private journal before any financial request,
preserving original inner errors and blocking replay until exact-ID local review.
Its `order(&client, side, &account, symbol, quantity, order_data).await` returns
`Result<anyhow::Result<OrderSubmission>, GuardError>`. No client/guard extraction,
public raw methods, legacy feature or public cancellation is available.
Read/authentication APIs are unchanged. Storage needs an explicit absolute path;
use the CLI's `BOURSO_DATA_DIR` or `~/.bourso` to share its Rust domain. DCA remains
separate. See [mandatory financial session](../../docs/LIBRARY_REPLAY_GUARD.md)
for signatures, Unix-only mutation support, review and no-fsync limits.
Provider obligations `bourso-y3u.4` and `bourso-y3u.8` remain open.

## Network/test boundary

Normal requests use real HTTPS bank/public-market URLs; some destinations come
from page configuration. There is no universal origin allowlist assertion.
Export redirects are specifically limited to `https://clients.boursobank.com`
on port 443 without URL credentials. Automatic redirects are disabled on the
main client. The private `cfg(test)` facade uses real reqwest requests to bound
loopback fixtures (HTTP, proxies/redirects disabled), not a production endpoint
override. These fixtures exercise shared method bodies, not all production
transport configuration or real bank behavior.
