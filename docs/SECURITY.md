# Security and sensitive data

This unofficial project handles credentials and financial data. Local operation,
passing tests and defensive parsing do **not** establish production readiness or
prevent financial loss. Read [known limitations](KNOWN_LIMITATIONS.md) before any
financial use. No MFA bypass or unattended financial automation is supported.

## Credentials and interaction

`config --username <customer_id>` saves the customer ID and a null password in
`~/.bourso/settings.json`, replacing prior settings. Authenticated commands read
the settings file or the root-level `--credentials <path>` JSON. Both use
`clientId` and optional `password` fields.

A missing/null password causes a hidden interactive prompt. That path does not
save the password. A non-null password in either JSON file is plaintext and is
used without a password prompt. The CLI's unconditional prompt-related message
is misleading for that case. No keychain encryption or memory-zeroization
promise is made. Credentials, cookies and tokens can exist in process memory.

Keep credentials out of shell arguments/history, environment dumps, repositories,
CI artifacts and reports. A credential file does not bypass bank MFA; device
approval/terminal input may still be necessary. The CLI's MFA polling deadline
is 300 seconds, not a timeout for the entire login or financial command.

## Managed files and retention

By default, the CLI manages the following files inside the user's `~/.bourso/`
directory, resolved using the platform's `UserDirs` home directory (including
Windows Known Folders). `BOURSO_DATA_DIR` overrides that managed directory for
settings reads/writes and logger initialization. It must be a **nonempty absolute
path**; empty, relative and NUL-containing values fail without falling back to the
user profile. The directory is created if absent; its parent must already exist.
Existing directories and files undergo the same private-storage checks as the
default location. No shell expansion is performed: `~` and `$HOME` are not expanded
by the application. An external `--credentials <path>` remains an independent,
read-only path and is not relocated by this override.

The environment is user-controlled: setting `BOURSO_DATA_DIR` authorizes local
storage and, on Unix, permission tightening at the chosen directory. Use a trusted
directory and ancestors owned/controlled by the intended user. This is not a
sandbox or protection against malicious same-UID environment/path manipulation,
and it does not control shell redirection. Windows inherited ACL restrictions
remain the caller's responsibility.

Managed files:

| File | Purpose / retention |
| --- | --- |
| `settings.json` | Customer ID and optional plaintext password; no automatic expiry |
| `bourso.log` | JSON diagnostic journal, at most 10 MiB |
| `bourso.log.1` | Sole rotated archive, at most 10 MiB |
| `bourso.log.lock` | Persistent file used for an exclusive active logger lock |

On Unix the directory is **0700**, managed regular files **0600**, independent of
umask. Existing managed permissions are tightened; symlinks, special files and
multiple hard links are refused. External credentials are read without changing
permissions: restrict them yourself. Use a trusted home directory and ancestors;
these checks are not a sandbox against malicious same-user processes racing
filesystem operations.

Rotation is checked on writes, replacing only the previous archive. It does not
purge settings or unrelated files. Oversized records and existing oversized logs
are refused, not silently truncated. Stop the CLI and securely move an oversized
journal/archive before retrying. Delete unneeded sensitive logs manually when no
logger is active. Removing a pathname is not a secure-erasure guarantee, and
backups may retain copies.

A second active logger using the same directory is refused. A lock file remaining
after exit is normal; its presence alone is not a held lock. Do not delete it to
bypass a running process. This protects journal rotation, **not financial
idempotency or DCA scheduling**.

Windows ACLs are neither configured nor tested by this Unix policy. Restrict the
user profile, credential files and export directories using appropriate local
ACLs. Do not assume Unix modes transfer to Windows.

## Diagnostics, terminal output and exports

The CLI console logger writes reviewed application diagnostics to **stderr**, with
verbosity controlled by `RUST_LOG`. The JSON file sink retains reviewed
application diagnostics through TRACE independently of `RUST_LOG`; setting a
quiet console level does not disable file retention. Dependency wire diagnostics
are excluded by the CLI's diagnostic filter even with verbose environment
settings. Authentication payloads, passwords, cookies and raw bank responses must
not be logged. Library consumers must configure their own sinks; the CLI's filter
is not automatically installed by `bourso_api`.

Redaction is not a claim that logs are public data: requested account output,
export contents, file paths, dates and some financial summaries remain sensitive.
Stdout also includes CLI presentation/progress; it is not universally a clean
machine-readable channel. Even help initializes log storage before Clap parsing.
Use an absolute temporary `BOURSO_DATA_DIR` and synthetic values for parser/help
checks. Changing `HOME` alone does not isolate Windows Known Folder resolution.

Export files use a sibling temporary file (0600 on Unix), synced before rename.
The chosen destination can be replaced on success. Failure before publication
preserves an existing destination. Parent-directory crash durability, Windows ACL
hardening and protection from malicious same-user filesystem races are not
promised. Stdout redirection is controlled by the shell, not this file policy.

**CSV formula injection:** fields starting with `=`, `+`, `-` or `@` are preserved.
Correct quoting prevents structural corruption but does not stop spreadsheet
execution. Import as text with formula interpretation disabled; never assume
untrusted CSV is safe to open normally. JSON is not a sanitization mechanism when
its values are later copied into a spreadsheet.

## Network origins and test isolation

Normal authenticated calls use `https://clients.boursobank.com` and URLs derived
from bank-page configuration, including API endpoints. Public quotes use
`https://www.boursorama.com`. The main client disables automatic redirects;
transaction export handles one redirect explicitly and permits only HTTPS,
`clients.boursobank.com`, port 443, with no URL username/password.

That export check is **not a universal outbound-origin allowlist**. Do not claim
all traffic can only reach the bank: configuration-derived destinations and
system proxy/network settings need separate consideration. Native TLS trust and
local host security remain dependencies. No live redirect/authentication
compatibility is asserted by local checks.

API HTTP unit tests use real HTTP requests to a bound loopback fixture through a
private `cfg(test)` facade. They prohibit external fallback and disable proxies
and redirects. This loopback HTTP exception is test-only, not a documented
production endpoint override or justification for disabling HTTPS. Shared
method-body tests do not verify the real service or every production client
setting. Cargo/CI dependency downloads are separate network operations.

## Financial safety and reporting

Do not infer execution from an HTTP status, progress message, order ID or DCA
timestamp. The check/confirm and transfer-receipt contracts remain insufficient;
a lost final response can leave an operation's outcome unknown. Stop and
independently reconcile with the bank before retrying. No provider deduplication
or reliable unknown-outcome recovery is established.

For a suspected vulnerability, provide a minimal synthetic reproduction and
source revision through an appropriate private maintainer channel, if available.
Do not post live credentials, account IDs, tokens or bank captures in public
issues. No dedicated disclosure channel, response SLA or bug bounty is promised
here. If data was exposed, stop sharing it and use the bank's official channels
for credential/account remediation; do not send secrets to this project.
