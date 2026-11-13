use crate::account::Transaction;
use crate::bank_money::{cents_to_euros, parse_cents};
use crate::constants::BASE_URL;

use super::BoursoWebClient;

use anyhow::{bail, Context, Result};
use lazy_static::lazy_static;
use regex::Regex;
use tracing::debug;

lazy_static! {
    static ref EXPORT_TOKEN_REGEX: Regex =
        Regex::new(r#"movementSearch\[_token\]"[^>]*?value="(?P<token>[^"]+)""#)
            .expect("Failed to compile export token regex");
}

/// URL of the page hosting the movement-export form (source of the CSRF token).
const EXPORT_FORM_URL: &str = "/mon-budget/generate";
/// Endpoint the export form POSTs to.
const EXPORT_SUBMIT_PATH: &str = "/budget/exporter-mouvements";

impl BoursoWebClient {
    /// Get the transactions for an account over a date range.
    ///
    /// Uses the BoursoBank CSV export endpoint to retrieve transactions.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account ID (32-character hex string).
    /// * `from_date` - Start date in DD/MM/YYYY format.
    /// * `to_date` - End date in DD/MM/YYYY format.
    ///
    /// # Returns
    ///
    /// The transactions list as a vector of `Transaction`.
    pub async fn get_transactions(
        &self,
        account_id: &str,
        from_date: &str,
        to_date: &str,
    ) -> Result<Vec<Transaction>> {
        let token = self.get_export_token().await?;

        let form: Vec<(&str, &str)> = vec![
            ("movementSearch[label]", ""),
            ("movementSearch[selectedAccounts][]", account_id),
            ("movementSearch[fromDate]", from_date),
            ("movementSearch[toDate]", to_date),
            ("movementSearch[format]", "CSV"),
            ("movementSearch[filtredBy]", "filtredByCategory"),
            ("movementSearch[category]", ""),
            ("movementSearch[operationTypes]", ""),
            ("movementSearch[myBudgetPage]", "1"),
            ("movementSearch[operationType]", ""),
            ("movementSearch[_token]", token.as_str()),
            ("movementSearch[submit]", ""),
        ];

        let response = self
            .client
            .post(format!("{BASE_URL}{EXPORT_SUBMIT_PATH}"))
            .headers(self.get_headers())
            .form(&form)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;

        // Follow redirects manually (the client uses Policy::none())
        let response = if response.status().is_redirection() {
            let location = response
                .headers()
                .get("location")
                .context("Missing redirect location header")?
                .to_str()
                .context("Invalid export redirect header")?;
            // Resolve against the original bank URL, not response.url(): the
            // test routing facade deliberately maps transport URLs to loopback.
            let redirect_url = resolve_export_redirect(location)?;
            debug!("Following export redirect");
            self.client
                .get(redirect_url)
                .headers(self.get_headers())
                .send()
                .await
                .map_err(reqwest::Error::without_url)?
        } else {
            response
        };

        let status = response.status();
        if !status.is_success() {
            bail!("Movement export failed with HTTP status {status}");
        }
        debug!("Export response status: {}", status);

        let res = response
            .bytes()
            .await
            .map_err(reqwest::Error::without_url)?;
        let content = std::str::from_utf8(&res).context("Movement export is not valid UTF-8")?;
        // Strip BOM if present
        let content = content.strip_prefix('\u{FEFF}').unwrap_or(content);

        // Only the verified CSV header can establish a legitimate empty export.
        // An HTML movement form may also mean rejection or an expired session.
        extract_transactions(content)
    }

    /// Fetch the export form page and scrape its `movementSearch[_token]` CSRF
    /// token, required to POST the movement export.
    async fn get_export_token(&self) -> Result<String> {
        let response = self
            .client
            .get(format!("{BASE_URL}{EXPORT_FORM_URL}"))
            .headers(self.get_headers())
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        let status = response.status();
        if !status.is_success() {
            bail!("Export form request failed with HTTP status {status}");
        }
        let page = response.text().await.map_err(reqwest::Error::without_url)?;
        extract_export_token(&page)
    }
}

fn resolve_export_redirect(location: &str) -> Result<reqwest::Url> {
    anyhow::ensure!(
        !location.trim().is_empty(),
        "Missing export redirect destination"
    );
    let base = reqwest::Url::parse(&format!("{BASE_URL}{EXPORT_SUBMIT_PATH}"))?;
    let url = base
        .join(location)
        .map_err(|_| anyhow::anyhow!("Invalid export redirect URL"))?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.host_str() == Some("clients.boursobank.com")
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none(),
        "Export redirect outside permitted HTTPS bank origin"
    );
    Ok(url)
}

/// Extract the `movementSearch[_token]` CSRF token from the export form page.
fn extract_export_token(page: &str) -> Result<String> {
    EXPORT_TOKEN_REGEX
        .captures(page)
        .and_then(|c| c.name("token"))
        .map(|m| m.as_str().to_string())
        .context(
            "Could not find movementSearch[_token] on the export form page — \
             the export form layout may have changed or the session expired.",
        )
}

/// Parse exact bank cents before crossing the public f64 compatibility boundary.
fn parse_amount(s: &str) -> Result<f64> {
    cents_to_euros(parse_cents(s)?)
}

/// Extract transactions from a BoursoBank CSV export string.
///
/// # Arguments
///
/// * `content` - The CSV content as a string (without BOM).
///
/// # Returns
///
/// The transactions list as a vector of `Transaction`.
fn extract_transactions(content: &str) -> Result<Vec<Transaction>> {
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b';')
        .has_headers(true)
        .flexible(false)
        .from_reader(content.as_bytes());

    // Positional mapping is supported only for this known schema. Never read
    // some other column as money; generalized header mapping is a separate task.
    const HEADERS: [&str; 11] = [
        "dateOp",
        "dateVal",
        "label",
        "category",
        "categoryParent",
        "supplierFound",
        "amount",
        "comment",
        "accountNum",
        "accountLabel",
        "accountbalance",
    ];
    let headers = reader
        .headers()
        .map_err(|_| anyhow::anyhow!("Failed to parse CSV headers"))?;
    if headers.len() != HEADERS.len() || !headers.iter().eq(HEADERS) {
        bail!("Unsupported or missing transaction CSV headers");
    }

    reader
        .records()
        .enumerate()
        .map(|(index, result)| {
            let row = index + 2;
            let record = result.map_err(|_| anyhow::anyhow!("Failed to parse CSV row {row}"))?;
            if record.len() != HEADERS.len() {
                bail!("Missing required transaction columns at CSV row {row}");
            }
            Ok(Transaction {
                date_op: record[0].to_string(),
                date_val: record[1].to_string(),
                label: record[2].to_string(),
                category: record[3].to_string(),
                category_parent: record[4].to_string(),
                supplier_found: record[5].to_string(),
                amount: parse_amount(&record[6])
                    .with_context(|| format!("Invalid transaction amount at CSV row {row}"))?,
                comment: record[7].to_string(),
                account_num: record[8].to_string(),
                account_label: record[9].to_string(),
                account_balance: parse_amount(&record[10])
                    .with_context(|| format!("Invalid account balance at CSV row {row}"))?,
            })
        })
        .collect()
}

/// Serialize transaction exports with CSV quoting and the existing eleven columns.
///
/// Spreadsheet policy: preserve every field literally, including leading `=`,
/// `+`, `-` and `@`. CSV quoting is NOT formula-injection protection. Import as
/// text when using spreadsheet software; no silent sanitization is performed.
pub fn transactions_to_csv(transactions: &[Transaction]) -> Result<String> {
    let mut writer = csv::WriterBuilder::new()
        .delimiter(b';')
        .from_writer(Vec::new());
    writer.write_record([
        "dateOp",
        "dateVal",
        "label",
        "category",
        "categoryParent",
        "supplierFound",
        "amount",
        "comment",
        "accountNum",
        "accountLabel",
        "accountbalance",
    ])?;
    for tx in transactions {
        writer.write_record([
            tx.date_op.as_str(),
            tx.date_val.as_str(),
            tx.label.as_str(),
            tx.category.as_str(),
            tx.category_parent.as_str(),
            tx.supplier_found.as_str(),
            &format!("{:.2}", tx.amount),
            tx.comment.as_str(),
            tx.account_num.as_str(),
            tx.account_label.as_str(),
            &format!("{:.2}", tx.account_balance),
        ])?;
    }
    let bytes = writer
        .into_inner()
        .map_err(|_| anyhow::anyhow!("Failed to flush CSV export"))?;
    String::from_utf8(bytes).context("Failed to encode CSV export")
}

/// Publish a fully validated/serialized export, replacing an explicitly selected
/// destination only after the sibling temporary file has been written and synced.
/// Failures before rename leave the previous destination untouched. Parent
/// directory durability across a system crash is not guaranteed.
pub fn write_export_atomically(path: &std::path::Path, content: &[u8]) -> Result<()> {
    publish_export(path, |file| {
        std::io::Write::write_all(file, content)?;
        Ok(())
    })
}

fn publish_export(
    path: &std::path::Path,
    write: impl FnOnce(&mut std::fs::File) -> Result<()>,
) -> Result<()> {
    use std::fs::OpenOptions;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    anyhow::ensure!(
        path.file_name().is_some(),
        "Export destination must name a file"
    );
    // create_new prevents following symlinks or clobbering somebody else's file.
    let (temp_path, mut file) = (0..128)
        .find_map(|_| {
            let temp_path = parent.join(format!(
                ".bourso-export-{}-{}.tmp",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&temp_path) {
                Ok(file) => Some(Ok((temp_path, file))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => None,
                Err(error) => Some(Err(error)),
            }
        })
        .context("Could not allocate export temporary file")??;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let _cleanup = Cleanup(temp_path.clone());
    write(&mut file)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temp_path, path).context("Could not publish export")?;
    Ok(())
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod export_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_export_token() {
        let page = r#"<form name="movementSearch" method="post" action="/budget/exporter-mouvements">
            <input type="hidden" name="movementSearch[myBudgetPage]" value="1" >
            <input id="movementSearch__token" type="hidden" name="movementSearch[_token]" value="test-csrf-token-not-a-real-value.1234abcd.5678efgh" >
            </form>"#;
        let token = extract_export_token(page).unwrap();
        assert_eq!(token, "test-csrf-token-not-a-real-value.1234abcd.5678efgh");
    }

    #[test]
    fn test_extract_export_token_missing() {
        assert!(extract_export_token("<html><body>no form here</body></html>").is_err());
    }

    #[test]
    fn test_parse_amount() {
        assert_eq!(parse_amount("-568,13").unwrap(), -568.13);
        assert_eq!(parse_amount("1 718,70").unwrap(), 1718.70);
        assert_eq!(parse_amount("-8,99").unwrap(), -8.99);
        assert_eq!(parse_amount("37.29").unwrap(), 37.29);
        assert!(parse_amount("").is_err());
        assert!(parse_amount("  ").is_err());
    }

    #[test]
    fn invalid_csv_amount_is_an_error() {
        let csv = TRANSACTIONS_CSV.replace("-568,13", "invalid");
        assert!(extract_transactions(&csv).is_err());
    }

    #[test]
    fn csv_monetary_fields_are_required_finite_and_bounded() {
        for original in ["-568,13", "37.29"] {
            for invalid in [
                "",
                " ",
                "NaN",
                "inf",
                "-inf",
                "1e2",
                "1 23,45",
                "12,345",
                "92233720368547758.08",
                "-92233720368547758.09",
                "22517998136852.48",
                "-22517998136852.48",
                "private-invalid-value",
            ] {
                let csv = TRANSACTIONS_CSV.replace(original, invalid);
                let diagnostic = format!("{:#}", extract_transactions(&csv).unwrap_err());
                assert!(diagnostic.contains("CSV row 2"));
                assert!(!diagnostic.contains("private-invalid-value"));
                assert!(!diagnostic.contains("00040613484"));
                assert!(!diagnostic.contains("Loyer Villard"));
            }
        }
    }

    #[test]
    fn csv_accepts_signed_grouping_and_integer_euros() {
        let csv = TRANSACTIONS_CSV
            .replace("-568,13", "−\u{202f}1\u{202f}234,5")
            .replace("37.29", "12");
        let transaction = &extract_transactions(&csv).unwrap()[0];
        assert_eq!(transaction.amount, -1234.5);
        assert_eq!(transaction.account_balance, 12.0);
    }

    #[test]
    fn csv_requires_known_headers_and_complete_rows() {
        for csv in [
            "".to_string(),
            TRANSACTIONS_CSV.replace(";amount;", ";wrong;"),
            TRANSACTIONS_CSV.replace(";accountbalance", ";amount"),
            TRANSACTIONS_CSV.replace("dateOp;dateVal", "dateVal;dateOp"),
            TRANSACTIONS_CSV.replace(";37.29", ""),
            TRANSACTIONS_CSV.replace(";37.29", ";37.29;extra"),
            TRANSACTIONS_CSV
                .lines()
                .skip(1)
                .collect::<Vec<_>>()
                .join("\n"),
        ] {
            assert!(extract_transactions(&csv).is_err());
        }
        // A valid header-only export is different from malformed/missing data.
        assert!(
            extract_transactions(TRANSACTIONS_CSV.lines().next().unwrap())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn test_extract_transactions() {
        let transactions = extract_transactions(TRANSACTIONS_CSV).unwrap();
        assert_eq!(transactions.len(), 3);
        assert_eq!(transactions[0].date_op, "2026-02-09");
        assert_eq!(transactions[0].label, "VIR SEPA Loyer Villard");
        assert_eq!(transactions[0].amount, -568.13);
        assert_eq!(transactions[0].account_balance, 37.29);
        assert_eq!(transactions[0].category, "Virements émis");
        assert_eq!(transactions[1].date_op, "2026-02-06");
        assert_eq!(
            transactions[1].label,
            "CARTE 05/02/26 AMZN Mktp FR*308J CB*7686"
        );
        assert_eq!(transactions[1].amount, -8.99);
        assert_eq!(transactions[2].label, "VIR SEPA FRANCE TRAVAIL");
        assert_eq!(transactions[2].amount, 1718.70);
        assert_eq!(transactions[2].account_balance, 629.41);
    }

    #[test]
    fn test_extract_transactions_empty_html() {
        let html = "<!DOCTYPE html><html><body>Error</body></html>";
        // HTML content should not be passed to extract_transactions
        // (handled by get_transactions), but let's verify it fails gracefully
        let result = extract_transactions(html);
        assert!(result.is_err());
    }

    pub const TRANSACTIONS_CSV: &str = r#"dateOp;dateVal;label;category;categoryParent;supplierFound;amount;comment;accountNum;accountLabel;accountbalance
2026-02-09;2026-02-09;"VIR SEPA Loyer Villard";"Virements émis";"Virements émis";"virement loyer villard";-568,13;;00040613484;BoursoBank;37.29
2026-02-06;2026-02-06;"CARTE 05/02/26 AMZN Mktp FR*308J CB*7686";"Livres, CD/DVD, bijoux, jouets…";"Vie quotidienne";amazon;-8,99;;00040613484;BoursoBank;605.42
2026-02-03;2026-02-03;"VIR SEPA FRANCE TRAVAIL";"Virements reçus";"Virements reçus";"virement france travail";1 718,70;;00040613484;BoursoBank;629.41
"#;
}
