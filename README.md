# Bourso CLI / API — maintenance fork

Unofficial Rust CLI and library for BoursoBank/Boursorama account access, public
quotes and transaction exports. Order and transfer commands also exist, but their
financial confirmation contracts are **not established as safe for production**.
This project is not affiliated with BoursoBank and provides no financial advice.

This checkout's remote is [bioinfornatics/bourso-api](https://github.com/bioinfornatics/bourso-api),
a fork of [azerpas/bourso-api](https://github.com/azerpas/bourso-api).
The Cargo repository metadata and CLI startup URL still name upstream. They do
not identify this fork's build, and no redirect or fork release availability is
implied. Upstream contributors include azerpas, Mathious6, mquandalle and
pierrediancourt. See [LICENSE](LICENSE) for the license terms.

> **Do not use DCA automation.** Financial success and safe automatic reconciliation are not established. See [DCA](#dca-dollar-cost-averaging) and
> [known limitations](docs/KNOWN_LIMITATIONS.md) before considering any financial
> operation. A successful local test is not proof of bank compatibility.

- [Build and installation](#build-and-installation)
- [Configuration and credentials](#configuration-and-credentials)
- [Commands](#commands)
- [Security and local files](#security-and-local-files)
- [Development](docs/DEVELOPMENT.md) · [API documentation](src/bourso_api/README.md)

## Build and installation

The supported minimum Rust version (MSRV) is **1.96.1**, declared by both crates
and pinned in `rust-toolchain.toml` with `rustfmt` and `clippy`. This is a
conservative supported minimum, not a claim that every earlier compiler fails:
file locking uses `std::fs::File::try_lock` (stable since 1.89), but lower
compiler support has not been established. Use the pinned toolchain for checks;
newer compiler compatibility is not a substitute for an MSRV run.

You need Cargo, a native linker/C toolchain and the TLS development dependencies
used by reqwest's default native-TLS backend (on Linux, typically OpenSSL headers
and pkg-config). See [development prerequisites](docs/DEVELOPMENT.md).

```sh
git clone https://github.com/bioinfornatics/bourso-api.git
cd bourso-api
cargo build --workspace --release --locked
./target/release/bourso-cli --help
```

Both crates share the root `Cargo.lock`. The executable is
`target/release/bourso-cli` (`bourso-cli.exe` on Windows), or `target/debug/` for a
debug build. The lockfile does not promise bit-for-bit reproducible artifacts.
Commands below use the source-built release binary from the repository root.

### Upstream downloads are not fork builds

The [upstream releases page](https://github.com/azerpas/bourso-api/releases)
is a separate distribution source; an upstream binary need not include this
fork's fixes. No published artifact of this fork is asserted here. If using a
download, check its source, platform, checksum and any available provenance
against that exact artifact and repository. Do not disable OS protections merely
to run an unverified binary. This document does not attest to a release or its
provenance.

## Configuration and credentials

Configure a customer ID locally; this command does not contact the bank:

```sh
./target/release/bourso-cli config --username 123456789
```

`123456789` is synthetic: substitute your own customer ID for actual use.
`--username` is required. This explicitly replaces `~/.bourso/settings.json`
with `clientId` and a null password; it does not ask for a password.

Authenticated commands read settings and prompt for a hidden password **only if
no password was supplied in those settings**. That prompt path does not save the
password. `config` and public `quote` commands do not read credentials or prompt.
The binary still initializes private log storage before parsing even `--help`.

An optional credentials JSON uses this shape (synthetic ID, no stored password):

```json
{"clientId":"123456789","password":null}
```

A non-null `password` string is a **plaintext secret** and suppresses the password
prompt. This applies to an explicitly supplied file and to a password manually
placed in the managed settings file. There is no encryption or keychain storage.
Place the root-level option **before** the subcommand:

```sh
./target/release/bourso-cli --credentials /private/path/credentials.json accounts
```

Do not put secrets in shell arguments, source control or bug reports. Protect
external credential files yourself; the CLI reads them without tightening their
permissions. The current startup message claiming a password is always prompted
is overbroad: the conditional behavior above is authoritative.

MFA may be required by the bank, including after network/IP changes; it is not
guaranteed for any particular circumstance. A password file does not bypass MFA.
The CLI's MFA polling phase has a 300-second deadline; this is not an end-to-end
command timeout. Interactive input/device approval can still be required.

## Commands

Use `--help` on each subcommand to inspect syntax without sending bank requests.
The following account/quote/export examples, unlike help/config, access the
network when run normally. IDs and dates below are illustrative, not bank fixtures.

### Accounts

```sh
./target/release/bourso-cli accounts
./target/release/bourso-cli accounts --trading
```

Optional boolean category flags are `--banking`, `--saving`, `--trading` and
`--loans`; select at most one. With none, all supported categories are requested.
Account output contains private IDs, balances and labels. A category retrieval
or parsing failure is an error, not a fabricated empty inventory.

### Public quotes (no login)

```sh
./target/release/bourso-cli quote --symbol 1rTCW8 --length 30 --interval 0 average
./target/release/bourso-cli quote --symbol 1rTCW8 --length 1 last
```

Operations: `highest`, `lowest`, `average`, `volume`, `last`; omitting the operation
lists history. Allowed `--length` values are exactly `1`, `5`, `30`, `90`, `180`,
`365`, `1825`, `3650` (default `30`). **`1M` is not accepted**, despite an obsolete
hint in CLI help. The only supported `--interval` value is `0` (default).

`last` uses the requested length/interval; it does **not** force a one-day fetch,
despite its current help text. It prefers a usable `qd` quote, otherwise the final
`QuoteTab` entry. It rejects non-finite OHLC prices and does not search earlier
history when the final entry is invalid. No usable quote yields an error.
Highest/lowest/average are fallible on empty/non-finite data; volume retains its
sum behavior (zero for empty history). Prices are not promised to be real-time.

### Transaction export

```sh
./target/release/bourso-cli export transactions \
  --account 0123456789abcdef0123456789abcdef \
  --start-date 01/01/2026 --end-date 31/01/2026 \
  --format csv --output /private/path/transactions.csv
```

Dates use `DD/MM/YYYY`; select `csv` (default) or `json`. Create a trusted private
parent directory first. Without `--output`, the serialized data goes to stdout,
which also contains CLI presentation output; do not assume a clean machine-data
pipe. Diagnostics normally go to stderr and the private JSON journal.

Exports fail closed on failed responses, unexpected HTML/schema, invalid UTF-8
or invalid money; they do not silently replace malformed amounts with zero or
publish a partial transaction list. A valid header-only CSV is a legitimate
empty export. CSV uses semicolons, eleven verified columns, and proper quoting.
The explicit output file is published only after complete validation,
serialization and a synced sibling temporary file. Failure before rename leaves
an existing destination intact; crash durability of its parent directory is not
guaranteed. Existing output may be replaced on success.

**Spreadsheet danger:** quoting is not formula protection. Fields beginning with
`=`, `+`, `-` or `@` are preserved literally. Import as text with formula evaluation
disabled; do not simply double-click an untrusted CSV. Exports remain sensitive.

### Orders and transfers: syntax, not an endorsement

**Do not use these commands as a production readiness test.** They can submit
real financial operations, with no safe dry-run or mandatory final human review
promised. Check/confirm results and transfer receipts remain inadequately
validated; apparent success is not proof of execution. After a lost response,
reconcile independently with the bank before any retry.

Inspect syntax safely:

```sh
./target/release/bourso-cli trade order new --help
./target/release/bourso-cli transfer --help
```

Order creation takes `--side buy|sell`, `--symbol`, `--account` and `--quantity`.
Results are explicit on stderr, even with logs off: exit `2` means this command
did not invoke its financial API; `3` means response received but unverified; `4`
means outcome unknown (possibly not emitted). Every invoked attempt retains a
private replay barrier until explicit operator review. See [the CLI guard and
review procedure](docs/FINANCIAL_GUARD.md). Library submissions now require
[`FinancialSession`](docs/LIBRARY_REPLAY_GUARD.md), with no public raw/legacy
financial methods or cancellation. API callers explicitly choose the same
absolute directory as the CLI to cooperate; DCA is a separate domain. Non-Unix
financial mutations refuse locally; read/authentication APIs remain available.

Quantity must be nonzero; the CLI uses bank-prefilled order options and exposes
no price-limit/expiration switches. Cancellation is not exposed by the CLI.

Transfer takes `--account`, `--to`, `--amount`, and optional `--reason`.
Amounts are unsigned ASCII decimal euros: `10`, `10.5`, `10,50` are valid input
spellings, not instructions to transfer. Minimum: **10 EUR**. At most two
fractional digits are accepted, without rounding (even `10.000` is rejected).
Whitespace, signs, exponents and grouping are forbidden. Validated amounts use
integer cents; the representation maximum is `184467440737095516.15` EUR,
**not a bank limit or permission**. Reason length currently counts UTF-8 bytes
up to 50, not 50 Unicode characters. See [limitations](docs/KNOWN_LIMITATIONS.md).

## DCA (Dollar Cost Averaging)

DCA was an original project goal. The current workflow is **semi-interactive**:
password input, a usable terminal and/or MFA device approval may be needed.
Credentials in a file do not make it safely unattended.

**DO NOT use `bourso-launchd.sh`, launchd, cron or Windows scheduled tasks for DCA
automation while provider contracts and trusted automatic success remain
unverified (`bourso-y3u.4`, `.6`, `.7`, `.8`).** The launcher now uses private attempt journaling, a concurrency lock and synchronous child observation, but exit zero is recorded as **submitted-unverified**, not financial success. Unknown attempts block repeats until explicit operator reconciliation. See [the assisted DCA procedure](docs/DCA.md). Trusted automatic success remains unavailable.

A launchd `RunAtLoad` trigger can add an immediate attempt to a calendar schedule,
creating duplicate-attempt risk. No runnable scheduler installation recipe is
provided while these defects remain. Previous unsafe permission and persistent
PowerShell session examples have been removed, not repaired into an automation
guarantee. Do not work around MFA or retry ambiguous operations automatically.

## Security and local files

Read [SECURITY.md](docs/SECURITY.md) for network boundaries, diagnostics and
reporting guidance. The application is local, but its network traffic is not
proven to be restricted globally to an allowlist of bank servers. Public quotes
use `www.boursorama.com`; authenticated flows use bank URLs/configuration.

### Local files and retention

The CLI manages `~/.bourso/settings.json`, `bourso.log`, `bourso.log.1`, and `bourso.log.lock`, and the CLI replay-barrier files described above. Financial
commands do not initialize diagnostic logging; this does not remove interactive
password/MFA requirements. On Unix, its directory is private (0700) and managed files are 0600, independently of umask. Existing managed regular files have their permissions tightened; symbolic links, special files and multiple hard links are refused. External credentials are read without changing their permissions: secure those files yourself.

The journal is limited to 10 MiB with one archive of at most 10 MiB (20 MiB total). Rotation does not purge configuration or unrelated files. An existing oversized journal is refused, not truncated: stop the CLI, move the journal/archive to a secure location and retry. A second active logger using the same directory is refused by an exclusive lock. A lock file left on disk is normal; its presence alone does not mean a process owns the lock.

Windows ACLs are not configured or tested by this Unix policy: restrict access to credentials and the data directory yourself. The directory and its ancestors must be trusted; this does not defend against malicious processes running as your own user. Diagnostics omit authentication data and dependency wire logs, but intentionally requested account/export output remains sensitive. Never share raw credentials or bank captures.

## Contributing and limitations

See [DEVELOPMENT.md](docs/DEVELOPMENT.md) for workspace checks and API-only
coverage, and [KNOWN_LIMITATIONS.md](docs/KNOWN_LIMITATIONS.md) for unresolved
financial and platform contracts. Submit only synthetic or reviewed anonymized
fixtures, never raw bank captures. Documentation updates do not establish bank
compatibility or close blocked financial work.

This software is provided as-is, without warranty. Use at your own risk; no
production-readiness or financial-loss protection is claimed.


### Candidate release artifacts

The revised workflow prepares draft releases only after its gates succeed. It has not been run remotely for this candidate. Expected assets are `bourso-cli-linux.tar.gz`, `bourso-cli-darwin.tar.gz`, and `bourso-cli-windows.zip` (containing `bourso-cli.exe`). Each archive includes `LICENSE`, `THIRD_PARTY_NOTICES.txt`, and `third-party-licenses/`. The conservative dependency inventory is not a legal certification or a substitute for system-library/source-offer obligations.

`SHA256SUMS` and `provenance.intoto.jsonl` refer to the **distribution archives**, not the extracted executables. When those assets actually exist, verify the checksum and SLSA provenance against the expected repository and tag; do not use an upstream release as proof for this fork. Example (requires a separately installed verifier):

```sh
slsa-verifier verify-artifact bourso-cli-linux.tar.gz --provenance-path provenance.intoto.jsonl --source-uri github.com/bioinfornatics/bourso-api --source-tag vX.Y.Z
```

This is an expected verification command, not evidence that a release or its provenance has already been verified.
