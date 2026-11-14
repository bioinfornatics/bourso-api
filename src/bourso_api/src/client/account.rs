use crate::{
    account::{Account, AccountKind},
    bank_money::parse_cents,
    constants::{ACCOUNT_PATTERN, ACCOUNT_SECTION_PATTERN, BASE_URL, HTML_TAG_PATTERN},
};

use super::BoursoWebClient;
use anyhow::{ensure, Context, Result};
use regex::Regex;

impl BoursoWebClient {
    /// Get accounts, optionally filtered by kind.
    ///
    /// HTTP failures, unrecognized pages and malformed account sections are errors,
    /// never partial or empty successful inventories. An absent category is empty
    /// only within a recognized account summary. No real-provider empty-portfolio
    /// layout without a known summary section is currently established.
    pub async fn get_accounts(&self, kind: Option<AccountKind>) -> Result<Vec<Account>> {
        let response = self
            .client
            .get(format!(
                "{BASE_URL}/dashboard/liste-comptes?rumroute=dashboard.new_accounts&_hinclude=1"
            ))
            .headers(self.get_headers())
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        ensure!(
            response.status().is_success(),
            "Account request failed with HTTP status {}",
            response.status().as_u16()
        );
        let page = response.text().await.map_err(reqwest::Error::without_url)?;
        let accounts = parse_accounts(&page)?;
        Ok(accounts
            .into_iter()
            .filter(|account| kind.is_none_or(|kind| account.kind == kind))
            .collect())
    }
}

// Retain the extraction seam used by the money regression tests. Validate the
// whole response before filtering, including malformed unrequested categories.
#[cfg(test)]
fn extract_accounts(page: &str, kind: AccountKind) -> Result<Vec<Account>> {
    Ok(parse_accounts(page)?
        .into_iter()
        .filter(|account| account.kind == kind)
        .collect())
}

/// Find a balanced element, rather than stopping at the first nested </ul>/</li>.
/// This deliberately recognizes only the fixture-backed summary list structure;
/// unexpected or truncated structure is an error, not HTML error recovery.
fn element_body<'a>(html: &'a str, tag: &str, tags: &Regex) -> Result<(&'a str, usize)> {
    let mut stack = Vec::new();
    let mut body_start = None;
    for token in tags.find_iter(html) {
        let text = token.as_str();
        if text.starts_with("<!--") {
            continue;
        }
        let name = text.trim_start_matches('<').trim_start_matches('/');
        let name = name
            .split(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .next()
            .unwrap_or("");
        if name != "ul" && name != "li" {
            continue;
        }
        if text.starts_with("</") {
            ensure!(stack.pop() == Some(name), "Malformed account list nesting");
            if stack.is_empty() {
                return Ok((
                    &html[body_start.context("Missing account section opening")?..token.start()],
                    token.end(),
                ));
            }
        } else {
            ensure!(
                !text.ends_with("/>"),
                "Malformed self-closing account section"
            );
            ensure!(
                name != "li" || stack.last() != Some(&"li"),
                "Malformed nested account rows"
            );
            if body_start.is_none() {
                ensure!(
                    token.start() == 0 && name == tag,
                    "Malformed account section opening"
                );
                body_start = Some(token.end());
            }
            stack.push(name);
        }
    }
    anyhow::bail!("Unclosed account section")
}

fn parse_accounts(page: &str) -> Result<Vec<Account>> {
    let tags = Regex::new(HTML_TAG_PATTERN)?;
    let section = Regex::new(ACCOUNT_SECTION_PATTERN)?;
    let account_regex = Regex::new(ACCOUNT_PATTERN)?;
    let list_item = Regex::new(r"(?i)^<li(?:\s|>)")?;
    // A login form is not a dashboard, even if stale summaries are present.
    let password_input = Regex::new(r#"(?i)^<input\b[^>]*\btype\s*=\s*["']?password(?:["'\s>])"#)?;
    let mut by_kind: [Vec<Account>; 4] = std::array::from_fn(|_| Vec::new());
    let mut seen = [false; 4];
    for token in tags.find_iter(page) {
        ensure!(
            !password_input.is_match(token.as_str()),
            "Account response contains a login form"
        );
        // Parse attributes as complete tokens so a marker inside a quoted
        // value (or a comment) cannot authenticate an unrelated page.
        if token.as_str().starts_with("<!--") {
            continue;
        }
        let mut attributes = section.captures_iter(token.as_str().trim_start_matches('<'));
        let tag_name = attributes.next().context("Malformed account HTML tag")?;
        let mut marker = None;
        for attribute in attributes {
            let name = &attribute[1];
            if matches!(
                name,
                "data-summary-savings"
                    | "data-summary-bank"
                    | "data-summary-trading"
                    | "data-summary-loan"
            ) {
                ensure!(marker.is_none(), "Ambiguous account summary category");
                marker = Some(attribute.get(1).unwrap().as_str());
            }
        }
        let Some(marker) = marker else {
            continue;
        };
        ensure!(
            &tag_name[1] == "ul" && !token.as_str().starts_with("</"),
            "Malformed account summary root"
        );
        let (index, kind) = match marker {
            "data-summary-savings" => (0, AccountKind::Savings),
            "data-summary-bank" => (1, AccountKind::Banking),
            "data-summary-trading" => (2, AccountKind::Trading),
            "data-summary-loan" => (3, AccountKind::Loans),
            _ => unreachable!("section regex only includes known categories"),
        };
        ensure!(!seen[index], "Duplicate account summary section");
        seen[index] = true;
        let (mut remaining, _) = element_body(&page[token.start()..], "ul", &tags)?;
        loop {
            remaining = remaining.trim_start();
            if remaining.is_empty() {
                break;
            }
            if remaining.starts_with("<!--") {
                let end = remaining
                    .find("-->")
                    .context("Unclosed account section comment")?;
                remaining = &remaining[end + 3..];
                continue;
            }
            ensure!(
                list_item.is_match(remaining),
                "Unexpected content in account section"
            );
            let (item, end) = element_body(remaining, "li", &tags)?;
            let mut matches = account_regex.captures_iter(item);
            let account = matches
                .next()
                .context("Malformed account row: missing required fields")?;
            ensure!(matches.next().is_none(), "Ambiguous account row");
            let field = |name| {
                account
                    .name(name)
                    .map(|value| value.as_str().trim())
                    .filter(|value| !value.is_empty())
                    .context("Missing required account field")
            };
            let cents = parse_cents(
                account
                    .name("balance")
                    .context("Missing account balance")?
                    .as_str()
                    .trim(),
            )
            .context("Invalid account balance")?;
            by_kind[index].push(Account {
                id: field("id")?.to_string(),
                name: field("name")?.to_string(),
                balance: isize::try_from(cents)
                    .context("Account balance exceeds platform cent range")?,
                bank_name: field("bank_name")?.to_string(),
                kind,
            });
            remaining = &remaining[end..];
        }
    }
    ensure!(
        seen.iter().any(|seen| *seen),
        "Unrecognized account response: missing account summary structure"
    );
    Ok(by_kind.into_iter().flatten().collect())
}

#[cfg(test)]
mod tests {
    use crate::{account::AccountKind, client::account::extract_accounts};

    #[test]
    fn integer_balance_is_euros() {
        let html = ACCOUNTS_RES.replace("11 010,00", "12");
        assert_eq!(
            extract_accounts(&html, AccountKind::Savings).unwrap()[0].balance,
            1200
        );
    }

    #[test]
    fn balances_accept_fractional_and_grouped_signed_euros() {
        for (amount, expected) in [
            ("12,3", 1230),
            ("12.30", 1230),
            ("− 1\u{202f}234,56", -123456),
            ("0", 0),
        ] {
            let html = ACCOUNTS_RES.replace("11 010,00", amount);
            assert_eq!(
                extract_accounts(&html, AccountKind::Savings).unwrap()[0].balance,
                expected
            );
        }
    }

    #[test]
    fn malformed_or_overflowing_balance_returns_safe_error() {
        for amount in [
            "NaN",
            "inf",
            "",
            "12,345",
            "1 23,45",
            "−−12",
            "92233720368547758.08",
            "-92233720368547758.09",
            "private-invalid-value",
        ] {
            let html = ACCOUNTS_RES.replace("11 010,00", amount);
            let error = extract_accounts(&html, AccountKind::Savings).unwrap_err();
            let diagnostic = format!("{error:#}");
            assert!(diagnostic.contains("account balance"));
            assert!(!diagnostic.contains("private-invalid-value"));
            assert!(!diagnostic.contains("a8a23172b7e7c91c538831578242112e"));
        }
    }

    #[test]
    fn test_extract_accounts() {
        let accounts = extract_accounts(ACCOUNTS_RES, AccountKind::Savings).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].name, "LIVRET DEVELOPPEMENT DURABLE SOLIDAIRE");
        assert_eq!(accounts[0].balance, 1101000);
        assert_eq!(accounts[0].bank_name, "BoursoBank");
        assert_eq!(accounts[1].id, "d4e4fd4067b6d4d0b538a15e42238ef9");
        assert_eq!(accounts[1].name, "Livret Jeune");
        assert_eq!(accounts[1].balance, 159972);
        assert_eq!(accounts[1].bank_name, "Crédit Agricole");
        let accounts = extract_accounts(ACCOUNTS_RES, AccountKind::Banking).unwrap();
        assert_eq!(accounts.len(), 2);
        assert_eq!(accounts[0].id, "e2f509c466f5294f15abd873dbbf8a62");
        assert_eq!(accounts[0].name, "BoursoBank");
        assert_eq!(accounts[0].balance, 2081050);
        assert_eq!(accounts[0].bank_name, "BoursoBank");
        assert_eq!(accounts[1].name, "Compte de chèques ****0102");
        assert_eq!(accounts[1].balance, 50040);
        assert_eq!(accounts[1].bank_name, "CIC");
        let accounts = extract_accounts(ACCOUNTS_RES, AccountKind::Trading).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].name, "PEA DOE");
        let accounts = extract_accounts(ACCOUNTS_RES, AccountKind::Loans).unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].name, "Prêt personnel");
        assert_eq!(accounts[0].balance, -9495982);
        assert_eq!(accounts[0].bank_name, "Crédit Agricole");
    }

    pub const ACCOUNTS_RES: &str = r#"<hx:include id="hinclude__XXXXXXXX" src="/dashboard/offres?rumroute=dashboard.offers"
    data-cs-override-id="dashboard.offers">
    <div class="c-offers_loading o-vertical-interval-bottom-medium">
        <div class="bourso-spinner">
            <img src=" data:image/png;base64,iVBO"
                alt="">
        </div>
    </div>
</hx:include>

<div class="c-panel c-panel--primary o-vertical-interval-bottom-medium " id="panel-XXXXXXXX">
    <div class="c-panel__header ">
        <span class="c-panel__title" id="panel-XXXXXXXX-title">
            Mon compte bancaire
        </span>
        <span class="c-panel__subtitle">
            21 310,90 €
        </span>
    </div>
    <div class="c-panel__body ">
        <div class="c-panel__no-animation-glitch ">
            <ul class="c-info-box " aria-label="Mon compte bancaire - Total : 21 310,90 €" role="list"
                data-brs-list-header data-summary-bank>
                <li class="c-panel__item c-info-box__item" data-brs-filterable>
                    <a class="c-info-box__link-wrapper" href="/compte/cav/e2f509c466f5294f15abd873dbbf8a62/"
                        data-tag-commander-click='{"label": "application::customer.dashboard::click_accounts_cav", "s2": 1, "type": "N"}'
                        aria-label="Détails du compte BoursoBank - Solde : 20 810,50 €" title="BoursoBank">

                        <span class="c-info-box__account">
                            <span class="c-info-box__account-label"
                                data-account-label="e2f509c466f5294f15abd873dbbf8a62" data-brs-list-item-label>
                                BoursoBank
                            </span>
                            <span class="c-info-box__account-balance c-info-box__account-balance--positive">
                                20 810,50 €
                            </span>
                        </span>

                        <span class="c-info-box__account-sub-label" data-brs-list-item-label>
                            BoursoBank
                        </span>

                        <ul class="c-info-box__account-attached-products">
                            <li class="c-info-box__product">
                                <span class="c-info-box__product-name">
                                    <span class="c-info-box__card ">
                                        <img class="c-info-box__card-image "
                                            src="/bundles/boursoramadesign/img/cbi/25x16/prime_black.png" alt=""
                                            aria-hidden="true">
                                    </span>
                                    JOHN DOE
                                </span>
                            </li>
                        </ul>
                    </a>
                </li>
                <li class="c-panel__item c-info-box__item" data-brs-filterable>
                    <a class="c-info-box__link-wrapper" href="/budget/compte/a22217240487004d13c8a6b5da422bbf/"
                        data-tag-commander-click='{"label": "application::customer.dashboard::click_accounts_pfm_cav", "s2": 1, "type": "N"}'
                        aria-label="Détails du compte Compte de chèques ****0102 - Solde : 500,40 €"
                        title="Compte de chèques ****0102">

                        <span class="c-info-box__account">
                            <span class="c-info-box__account-label"
                                data-account-label="a22217240487004d13c8a6b5da422bbf" data-brs-list-item-label>
                                Compte de chèques ****0102
                            </span>
                            <span class="c-info-box__account-balance c-info-box__account-balance--positive">
                                500,40 €
                            </span>
                        </span>

                        <span class="c-info-box__account-sub-label" data-brs-list-item-label>
                            CIC
                        </span>
                    </a>
                </li>
            </ul>
        </div>
    </div>
</div>


<div class="c-panel c-panel--primary o-vertical-interval-bottom-medium " id="panel-XXXXXXXX">
    <div class="c-panel__header ">
        <span class="c-panel__title" id="panel-XXXXXXXX-title">
            Mon épargne
        </span>
        <span class="c-panel__subtitle">
            12 609,72 €
        </span>
    </div>
    <div class="c-panel__body ">
        <div class="c-panel__no-animation-glitch ">
            <ul class="c-info-box " aria-label="Mon épargne - Total : 12 609,72 €" role="list" data-brs-list-header
                data-summary-savings>
                <li class="c-panel__item c-info-box__item" data-brs-filterable>
                    <a class="c-info-box__link-wrapper" href="/compte/epargne/ldd/a8a23172b7e7c91c538831578242112e/"
                        data-tag-commander-click='{"label": "application::customer.dashboard::click_accounts_saving", "s2": 1, "type": "N"}'
                        aria-label="Détails du compte LIVRET DEVELOPPEMENT DURABLE SOLIDAIRE - Solde : 11 010,00 €"
                        title="LIVRET DEVELOPPEMENT DURABLE SOLIDAIRE">

                        <span class="c-info-box__account">
                            <span class="c-info-box__account-label"
                                data-account-label="a8a23172b7e7c91c538831578242112e" data-brs-list-item-label>
                                LIVRET DEVELOPPEMENT DURABLE SOLIDAIRE
                            </span>
                            <span class="c-info-box__account-balance c-info-box__account-balance--positive">
                                11 010,00 €
                            </span>
                        </span>

                        <span class="c-info-box__account-sub-label" data-brs-list-item-label>
                            BoursoBank
                        </span>
                    </a>
                </li>
                <li class="c-panel__item c-info-box__item" data-brs-filterable>
                    <a class="c-info-box__link-wrapper" href="/budget/compte/d4e4fd4067b6d4d0b538a15e42238ef9/"
                        data-tag-commander-click='{"label": "application::customer.dashboard::click_accounts_pfm_saving", "s2": 1, "type": "N"}'
                        aria-label="Détails du compte Livret Jeune - Solde : 1 599,72 €" title="Livret Jeune">

                        <span class="c-info-box__account">
                            <span class="c-info-box__account-label"
                                data-account-label="d4e4fd4067b6d4d0b538a15e42238ef9" data-brs-list-item-label>
                                Livret Jeune
                            </span>
                            <span class="c-info-box__account-balance c-info-box__account-balance--positive">
                                1 599,72 €
                            </span>
                        </span>

                        <span class="c-info-box__account-sub-label" data-brs-list-item-label>
                            Crédit Agricole
                        </span>
                    </a>
                </li>
            </ul>
        </div>
    </div>
</div>


<div class="c-panel c-panel--primary o-vertical-interval-bottom-medium " id="panel-XXXXXXXX">
    <div class="c-panel__header ">
        <span class="c-panel__title" id="panel-XXXXXXXX-title">
            Mes placements financiers
        </span>
        <span class="c-panel__subtitle">
            143 088,89 €
        </span>
    </div>
    <div class="c-panel__body ">
        <div class="c-panel__no-animation-glitch ">
            <ul class="c-info-box " aria-label="Mes placements financiers - Total : 143 088,89 €" role="list"
                data-brs-list-header data-summary-trading>
                <li class="c-panel__item c-info-box__item" data-brs-filterable>
                    <a class="c-info-box__link-wrapper" href="/compte/pea/9651d8edd5975de1b9eff3865505f15f/"
                        data-tag-commander-click='{"label": "application::customer.dashboard::click_accounts_investement", "s2": 1, "type": "N"}'
                        aria-label="Détails du compte PEA DOE - Solde : 143 088,89 €" title="PEA DOE">

                        <span class="c-info-box__account">
                            <span class="c-info-box__account-label"
                                data-account-label="9651d8edd5975de1b9eff3865505f15f" data-brs-list-item-label>
                                PEA DOE
                            </span>
                            <span class="c-info-box__account-balance c-info-box__account-balance--positive">
                                143 088,89 €
                            </span>
                        </span>

                        <span class="c-info-box__account-sub-label" data-brs-list-item-label>
                            BoursoBank
                        </span>
                    </a>
                </li>
            </ul>
        </div>
    </div>
</div>


<div class="c-panel c-panel--primary o-vertical-interval-bottom-medium " id="panel-XXXXXXXX">
    <div class="c-panel__header ">
        <span class="c-panel__title" id="panel-XXXXXXXX-title">
            Mes crédits
        </span>
        <span class="c-panel__subtitle">
            − 94 959,82 €
        </span>
    </div>
    <div class="c-panel__body ">
        <div class="c-panel__no-animation-glitch ">
            <ul class="c-info-box " aria-label="Mes crédits - Total : − 94 959,82 €" role="list" data-brs-list-header
                data-summary-loan>
                <li class="c-panel__item c-info-box__item" data-brs-filterable>
                    <a class="c-info-box__link-wrapper" href="/budget/compte/7315a57115ae889992ec98a6bb3571cb/"
                        data-tag-commander-click='{"label": "application::customer.dashboard::click_accounts_pfm_loan", "s2": 1, "type": "N"}'
                        aria-label="Détails du compte Prêt personnel - Solde : − 94 959,82 €" title="Prêt personnel">

                        <span class="c-info-box__account">
                            <span class="c-info-box__account-label"
                                data-account-label="7315a57115ae889992ec98a6bb3571cb" data-brs-list-item-label>
                                Prêt personnel
                            </span>
                            <span class="c-info-box__account-balance c-info-box__account-balance--neutral">
                                − 94 959,82 €
                            </span>
                        </span>

                        <span class="c-info-box__account-sub-label" data-brs-list-item-label>
                            Crédit Agricole
                        </span>
                    </a>
                </li>
            </ul>
        </div>
    </div>
</div>

<!-- The Corner -->

<!-- Ajouter un compte externe -->

<!-- script -->
    "#;
}

#[cfg(test)]
#[path = "newaccount_tests.rs"]
mod newaccount_tests;
