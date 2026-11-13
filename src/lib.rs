use anyhow::{Context, Result};
use bourso_api::{
    account::{Account, AccountKind, Transaction},
    client::{
        trade::{order::OrderSide, tick::GetTicksEOD},
        transfer::TransferProgress,
        BoursoWebClient,
    },
    get_client,
};
use clap::ArgMatches;
use futures_util::{pin_mut, StreamExt};
use tracing::{debug, info, warn};

pub mod account_filters;
pub mod financial_guard;
mod mfa;
pub mod money;
pub mod qrcode;
pub mod settings;
pub mod validate;

use settings::{get_settings, save_settings, Settings};

#[cfg(not(tarpaulin_include))]
pub async fn parse_matches(matches: ArgMatches) -> Result<()> {
    parse_matches_with_dependencies(
        matches,
        |matches| match matches.get_one::<String>("credentials") {
            Some(path) => Settings::load(path),
            None => get_settings(),
        },
        |symbol, length, interval| async move {
            get_client().get_ticks(&symbol, length, interval).await
        },
        || Ok(get_client()),
    )
    .await
}

// Private injection seam: tests cannot redirect the production client's endpoints.
#[cfg(not(tarpaulin_include))]
async fn parse_matches_with_dependencies<L, F, Fut, B>(
    matches: ArgMatches,
    load_settings: L,
    fetch_quotes: F,
    bank_client: B,
) -> Result<()>
where
    L: FnOnce(&ArgMatches) -> Result<Settings>,
    F: FnOnce(String, i64, i64) -> Fut,
    Fut: std::future::Future<Output = Result<GetTicksEOD>>,
    B: FnOnce() -> Result<BoursoWebClient>,
{
    if let Some(("financial-review", review)) = matches.subcommand() {
        return financial_guard::review(review);
    }
    let financial = matches!(matches.subcommand_name(), Some("trade" | "transfer"));

    // Validate even caller-built ArgMatches before settings, credentials or network.
    let transfer_amount = matches
        .subcommand_matches("transfer")
        .map(|transfer| -> Result<money::Money> {
            Ok(transfer
                .get_one::<String>("amount")
                .context("Missing transfer amount")?
                .parse()?)
        })
        .transpose()?;

    info!("Welcome to BoursoBank CLI 👋");
    info!(
        "ℹ️ - Version {}. Make sure you're running the latest version: {}",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_REPOSITORY")
    );
    println!();

    match matches.subcommand() {
        // These matches do not require authentication
        Some(("config", config_matches)) => {
            let customer_id = config_matches
                .get_one::<String>("username")
                .map(|s| s.as_str())
                .unwrap();
            save_settings(&Settings {
                customer_id: Some(customer_id.to_string()),
                password: None,
            })?;
            info!("Configuration saved ✅");
            return Ok(());
        }
        Some(("quote", quote_matches)) => {
            info!("Fetching quotes...");

            let symbol = quote_matches
                .get_one::<String>("symbol")
                .map(|s| s.as_str())
                .unwrap();
            let length = quote_matches
                .get_one::<String>("length")
                .map(|s| s.as_str())
                .unwrap();
            let interval = quote_matches
                .get_one::<String>("interval")
                .map(|s| s.as_str())
                .unwrap();
            let quotes =
                fetch_quotes(symbol.to_owned(), length.parse()?, interval.parse()?).await?;

            match quote_matches.subcommand() {
                Some(("highest", _)) => {
                    let highest_quote = quotes.d.get_highest_value()?;
                    info!(highest_quote, "Highest quote: {:#?}", highest_quote);
                }
                Some(("lowest", _)) => {
                    let lowest_quote = quotes.d.get_lowest_value()?;
                    info!(lowest_quote, "Lowest quote: {:#?}", lowest_quote);
                }
                Some(("volume", _)) => {
                    let volume = quotes.d.get_volume();
                    info!(volume, "Volume: {:#?}", volume);
                }
                Some(("average", _)) => {
                    let average_quote = quotes.d.get_average_value()?;
                    info!(average_quote, "Average quote: {:#?}", average_quote);
                }
                Some(("last", _)) => {
                    let quote = quotes
                        .d
                        .get_latest_quote()
                        .context("No quotation available for this symbol and period")?;

                    info!(
                        close = quote.close, open = quote.open, high = quote.high, low = quote.low, volume = quote.volume,
                        "Last quote: current: {:#?}, open: {:#?}, high: {:#?}, low: {:#?}, volume: {:#?}",
                        quote.close, quote.open, quote.high, quote.low, quote.volume
                    );
                }
                _ => {
                    info!("Quotes:");
                    for quote in quotes.d.quote_tab.iter() {
                        info!(
                            date = quote.date, close = quote.close, open = quote.open, high = quote.high, low = quote.low, volume = quote.volume,
                            "Quote day {:#?}: Close: {:#?}, Open: {:#?}, High: {:#?}, Low: {:#?}, Volume: {:#?}",
                            quote.date, quote.close, quote.open, quote.high, quote.low, quote.volume,
                        );
                    }
                }
            }

            return Ok(());
        }
        // These matches require authentication
        Some(("accounts", _))
        | Some(("export", _))
        | Some(("balance", _))
        | Some(("trade", _))
        | Some(("transfer", _)) => (),
        _ => unreachable!(),
    }

    // Only authenticated commands load credentials or prompt for a password.
    let settings = load_settings(&matches)?;
    let customer_id = settings.customer_id.context(
        "Missing customer id. Please configure it with `bourso config --username <customer_id>`",
    )?;

    let session = if financial {
        Some(financial_guard::Session::open()?)
    } else {
        None
    };

    info!("Authenticating with the configured customer id");
    info!("If you want to change it, run `bourso config --username <customer_id>`");
    println!();
    info!("We'll need your password to log you in. It will not be stored anywhere and will be asked everytime you run a command. The password will be hidden while typing.");

    // Get password from stdin
    let password = match settings.password {
        Some(password) => password,
        None => rpassword::prompt_password("Enter your password: ")
            .context("Failed to read password")?
            .trim()
            .to_string(),
    };

    let mut web_client = bank_client()?;
    web_client.init_session().await?;
    match web_client.login(&customer_id, &password).await {
        Ok(_) => {
            info!("Login successful ✅");
        }
        Err(e) => match e.downcast_ref() {
            Some(bourso_api::client::error::ClientError::MfaRequired) => {
                warn!("An MFA is required.");
                if financial {
                    println!("Authentication requires device approval; open the bank app. No financial API invoked yet.");
                }

                mfa::within_deadline(mfa::VALIDATION_TIMEOUT, async {
                    let (otp_id, form_state, token, mfa_type) = web_client.request_mfa().await?;
                    info!("To validate your identity, please open the BoursoBank app and validate the login request.");
                    mfa::wait_until(mfa::POLL_INTERVAL, async || {
                        let response = web_client.check_mfa(
                            mfa_type.clone(), otp_id.clone(), form_state.clone(), token.clone(),
                        ).await;
                        mfa::validation_status(response, |code| {
                            let qr = qrcode::generate_qr_code(code)?;
                            println!("\n{}\n", qrcode::render_to_terminal(&qr));
                            info!("Please scan the latest QR code in your BoursoBank app.");
                            Ok(())
                        })
                    }).await
                }).await?;

                info!("MFA successful ✅");
            }
            _ => {
                if !financial {
                    debug!("{:#?}", e);
                }
                return Err(e);
            }
        },
    }

    let accounts: Vec<Account>;

    match matches.subcommand() {
        Some(("accounts", sub_matches)) => {
            let category = account_filters::selected(sub_matches).map(|filter| match filter {
                account_filters::AccountFilter::Banking => AccountKind::Banking,
                account_filters::AccountFilter::Savings => AccountKind::Savings,
                account_filters::AccountFilter::Trading => AccountKind::Trading,
                account_filters::AccountFilter::Loans => AccountKind::Loans,
            });
            accounts = web_client.get_accounts(category).await?;

            info!("Found {} accounts", accounts.len());
            println!("{:#?}", accounts);
        }

        Some(("export", export_matches)) => match export_matches.subcommand() {
            Some(("transactions", tx_matches)) => {
                let account_id = tx_matches
                    .get_one::<String>("account")
                    .map(|s| s.as_str())
                    .unwrap();
                let start_date = tx_matches
                    .get_one::<String>("start-date")
                    .map(|s| s.as_str())
                    .unwrap();
                let end_date = tx_matches
                    .get_one::<String>("end-date")
                    .map(|s| s.as_str())
                    .unwrap();
                let format = tx_matches
                    .get_one::<String>("format")
                    .map(|s| s.as_str())
                    .unwrap();
                let output_path = tx_matches.get_one::<String>("output").map(|s| s.as_str());

                info!(
                    "Fetching transactions from {} to {}...",
                    start_date, end_date
                );

                let transactions: Vec<Transaction> = web_client
                    .get_transactions(account_id, start_date, end_date)
                    .await?;

                info!("Found {} transactions", transactions.len());

                let content = match format {
                    "json" => serde_json::to_string_pretty(&transactions)?,
                    _ => bourso_api::client::transaction::transactions_to_csv(&transactions)?,
                };

                match output_path {
                    Some(path) => {
                        bourso_api::client::transaction::write_export_atomically(
                            std::path::Path::new(path),
                            content.as_bytes(),
                        )?;
                        info!("Transactions exported to {}", path);
                    }
                    None => {
                        println!("{}", content);
                    }
                }
            }
            _ => unreachable!(),
        },

        Some(("trade", trade_matches)) => {
            accounts = web_client.get_accounts(Some(AccountKind::Trading)).await?;

            match trade_matches.subcommand() {
                Some(("order", order_matches)) => {
                    match order_matches.subcommand() {
                        Some(("new", new_order_matches)) => {
                            let account_id = new_order_matches
                                .get_one::<String>("account")
                                .map(|s| s.as_str())
                                .unwrap();

                            // Get account from previously fetched accounts
                            let account = accounts
                                .iter()
                                .find(|a| a.id == account_id)
                                .context("Account not found. Are you sure you have access to it? Run `bourso accounts` to list your accounts")?;

                            let side = new_order_matches.get_one::<OrderSide>("side").unwrap();
                            let quantity = new_order_matches.get_one::<usize>("quantity").unwrap();
                            let symbol = new_order_matches
                                .get_one::<String>("symbol")
                                .map(|s| s.as_str())
                                .unwrap();

                            let report = session
                                .as_ref()
                                .context("Missing financial guard")?
                                .invoke(financial_guard::Operation::Order, || async {
                                    web_client
                                        .order_detailed(
                                            side.to_owned(),
                                            account,
                                            symbol,
                                            *quantity,
                                            None,
                                        )
                                        .await?;
                                    Ok(())
                                })
                                .await;
                            return Err(report.into());
                        }
                        _ => unreachable!(),
                    }
                }
                _ => unreachable!(),
            }
        }

        Some(("transfer", transfer_matches)) => {
            accounts = web_client.get_accounts(None).await?;

            let from_account_id = transfer_matches
                .get_one::<String>("account")
                .map(|s| s.as_str())
                .unwrap();
            let to_account_id = transfer_matches
                .get_one::<String>("to_account")
                .map(|s| s.as_str())
                .unwrap();
            let amount = transfer_amount.context("Missing validated transfer amount")?;
            let reason = transfer_matches
                .get_one::<String>("reason")
                .map(|s| s.as_str());

            // Get from_account from previously fetched accounts
            let from_account = accounts
                .iter()
                .find(|a| a.id == from_account_id)
                .context("From account not found. Are you sure you have access to it? Run `bourso accounts` to list your accounts")?;

            // Get to_account from previously fetched accounts
            let to_account = accounts
                .iter()
                .find(|a| a.id == to_account_id)
                .context("To account not found. Are you sure you have access to it? Run `bourso accounts` to list your accounts")?;

            let report = session
                .as_ref()
                .context("Missing financial guard")?
                .invoke(financial_guard::Operation::Transfer, || async {
                    let stream = web_client.transfer_money(
                        amount,
                        from_account.clone(),
                        to_account.clone(),
                        reason.map(str::to_owned),
                    );
                    pin_mut!(stream);
                    let mut terminal_response = false;
                    while let Some(progress) = stream.next().await {
                        terminal_response =
                            matches!(progress?, TransferProgress::SubmittedUnverified);
                    }
                    anyhow::ensure!(terminal_response, "No terminal transfer response");
                    Ok(())
                })
                .await;
            return Err(report.into());
        }

        _ => unreachable!(),
    }

    Ok(())
}

#[cfg(all(test, not(tarpaulin_include)))]
mod dispatch_tests {
    use super::*;
    use bourso_api::client::trade::tick::{QuoteTab, D};
    use clap::{Arg, Command};
    use std::cell::Cell;

    fn quote_matches(operation: &str) -> ArgMatches {
        Command::new("bourso")
            .arg(Arg::new("credentials").long("credentials"))
            .subcommand(Command::new("transfer"))
            .subcommand(
                Command::new("quote")
                    .arg(Arg::new("symbol").long("symbol"))
                    .arg(Arg::new("length").default_value("30"))
                    .arg(Arg::new("interval").default_value("0"))
                    .subcommands(
                        ["last", "highest", "lowest", "average", "volume"].map(Command::new),
                    ),
            )
            .try_get_matches_from([
                "bourso",
                "--credentials",
                "/unreadable/credentials",
                "quote",
                "--symbol",
                "fixture",
                operation,
            ])
            .unwrap()
    }

    #[tokio::test]
    async fn public_quotes_never_load_settings_and_use_only_injected_data() {
        for operation in ["last", "highest", "lowest", "average", "volume"] {
            let fetched = Cell::new(false);
            parse_matches_with_dependencies(
                quote_matches(operation),
                |_| panic!("public quote must not load settings or credentials"),
                |symbol, length, interval| {
                    fetched.set(true);
                    assert_eq!((symbol.as_str(), length, interval), ("fixture", 30, 0));
                    async {
                        Ok(GetTicksEOD {
                            d: D {
                                quote_tab: vec![QuoteTab {
                                    high: 3.0,
                                    low: 1.0,
                                    close: 2.0,
                                    ..Default::default()
                                }],
                                ..Default::default()
                            },
                        })
                    }
                },
                || panic!("public quote must not construct a bank client"),
            )
            .await
            .unwrap();
            assert!(fetched.get());
        }
    }

    #[tokio::test]
    async fn empty_quotes_return_errors_without_panicking() {
        for operation in ["highest", "lowest", "average", "last"] {
            let result = parse_matches_with_dependencies(
                quote_matches(operation),
                |_| panic!("quote unexpectedly loaded settings"),
                |_, _, _| async { Ok(GetTicksEOD::default()) },
                || panic!("public quote must not construct a bank client"),
            )
            .await;
            assert!(result.is_err(), "empty {operation} was accepted");
        }
    }

    #[tokio::test]
    async fn missing_customer_id_returns_before_any_quote_or_bank_work() {
        for name in ["accounts", "export", "trade", "transfer"] {
            let matches = Command::new("bourso")
                .subcommands(["accounts", "export", "trade"].map(Command::new))
                .subcommand(Command::new("transfer").arg(Arg::new("amount").default_value("10")))
                .try_get_matches_from(["bourso", name])
                .unwrap();
            let result = parse_matches_with_dependencies(
                matches,
                |_| {
                    Ok(Settings {
                        customer_id: None,
                        password: None,
                    })
                },
                |_, _, _| async { panic!("unexpected quote HTTP request") },
                || panic!("missing customer id must not construct a bank client"),
            )
            .await;
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("config --username"));
        }
    }
    #[tokio::test]
    async fn valid_configuration_reaches_bank_boundary_without_real_operation() {
        let matches = Command::new("bourso")
            .subcommand(Command::new("transfer"))
            .subcommand(Command::new("accounts"))
            .try_get_matches_from(["bourso", "accounts"])
            .unwrap();
        let result = parse_matches_with_dependencies(
            matches,
            |_| {
                Ok(Settings {
                    customer_id: Some("fixture".into()),
                    password: Some("synthetic".into()),
                })
            },
            |_, _, _| async { panic!("bank command must not fetch quotes") },
            || anyhow::bail!("injected bank boundary reached; no HTTP performed"),
        )
        .await;
        assert_eq!(
            result.unwrap_err().to_string(),
            "injected bank boundary reached; no HTTP performed"
        );
    }

    #[tokio::test]
    async fn last_accepts_qd_without_history() {
        parse_matches_with_dependencies(
            quote_matches("last"),
            |_| panic!("quote must not read settings"),
            |_, _, _| async {
                Ok(GetTicksEOD {
                    d: D {
                        last_quote: Some(QuoteTab {
                            close: 7.0,
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                })
            },
            || panic!("quote must not construct bank client"),
        )
        .await
        .unwrap();
    }
}
