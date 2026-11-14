use anyhow::Result;
use bourso_api::client::trade::order::OrderSide;
use bourso_cli::{settings::init_logger, validate::validate_account_id};
use clap::{
    builder::{PossibleValue, ValueParser},
    Arg, Command,
};

#[tokio::main]
async fn main() -> Result<()> {
    const VERSION: Option<&str> = option_env!("CARGO_PKG_VERSION");

    let account_arg = Arg::new("account")
        .short('a')
        .long("account")
        .help("The account to use by its 'id' (e.g: 'e51f635524a7d506e4d4a7a8088b6278'). You can get this info with the command `bourso accounts`")
        .value_parser(clap::value_parser!(String)) // Enforce input as String
        .value_parser(ValueParser::new(validate_account_id))
        .required(true);

    let args: Vec<_> = std::env::args_os().collect();
    // Even clap diagnostics can echo invalid financial values; suppress them.
    let sensitive = args
        .iter()
        .any(|arg| arg == "trade" || arg == "transfer" || arg == "financial-review");
    if !sensitive {
        init_logger()?;
    }
    let matches = Command::new("bourso")
        .version(VERSION.unwrap_or("0.0.1"))
        .author("@azerpas")
        .about("BoursoBank/Boursorama CLI")
        .subcommand_required(true)
        .arg_required_else_help(true)
        .subcommand(bourso_cli::account_filters::command())
        .subcommand(bourso_cli::financial_guard::review_command())
        .subcommand(
            Command::new("config")
                .about("Configure BoursoBank/Boursorama CLI")
                .arg(
                    Arg::new("username")
                        .short('u')
                        .long("username")
                        .help("Your customer id")
                        .required(true)
                )
        )
        // .subcommand( // interactive mode
        .subcommand(
            Command::new("trade")
                .about("Trade with your accounts")
                .subcommand(
                    Command::new("order")
                    // .subcommand(
                    //    Command::new("list")
                    //        .about("List all your current orders")
                    //        .arg(account_arg.clone())
                    // )
                    .subcommand(
                        Command::new("new")
                            .about("Place a new order")
                            .arg(
                                Arg::new("side")
                                .long("side")
                                .help("The side of the order (buy/sell)")
                                .required(true)
                                .value_parser(clap::value_parser!(OrderSide))
                            )
                            .arg(account_arg.clone())
                            .arg(
                                Arg::new("symbol")
                                .long("symbol")
                                .help("The symbol id of the order (e.g: '1rTCW8')")
                                .required(true)
                            )
                            .arg(
                                Arg::new("quantity")
                                .short('q')
                                .long("quantity")
                                .help("The quantity of the order (e.g: '1')")
                                .required(true)
                                .value_parser(clap::value_parser!(usize))
                            )
                            // Price limit
                            // Validity date
                            // TODO: handle other types of orders
                    )
                    // .subcommand(
                    //    Command::new("cancel")
                    //        .about("Cancel an order")
                    //        .arg(account_arg.clone())
                    // )
                    .subcommand_required(true)
                )
                .subcommand_required(true)
        )
        .subcommand(
            Command::new("quote")
                .about("Get quote details for a given symbol over a timeframe. This action does not require authentication")
                // subcommand highest value etc...
                .arg(
                    Arg::new("symbol")
                    .long("symbol")
                    .help("The symbol id of the stock (e.g: '1rTCW8')")
                    .required(true)
                )
                .arg(
                    Arg::new("length")
                    .long("length")
                    .help("The length period of the stock (e.g: '30' or '1M' for 30 days)")
                    .default_value("30")
                    .value_parser([
                        PossibleValue::new("1").help("1 day"),
                        PossibleValue::new("5").help("5 days"),
                        PossibleValue::new("30").help("30 days or 1 month"),
                        PossibleValue::new("90").help("90 days or 3 months"),
                        PossibleValue::new("180").help("180 days or 6 months"),
                        PossibleValue::new("365").help("365 days or 1 year"),
                        PossibleValue::new("1825").help("1825 days or 5 years"),
                        PossibleValue::new("3650").help("3650 days or 10 years"),
                    ])
                )
                .arg(
                    Arg::new("interval")
                    .long("interval")
                    .help("The interval of the stock (e.g: '0' for default interval)")
                    .default_value("0")
                    .value_parser([
                        PossibleValue::new("0").help("Default interval")
                        // TODO: add other intervals by documenting them on Boursorama
                    ])
                )
                .subcommand(
                    Command::new("highest")
                        .about("Get the highest value of the stock for the given period (length) and interval")
                )
                .subcommand(
                    Command::new("lowest")
                        .about("Get the lowest value of the stock for the given period (length) and interval")
                )
                .subcommand(
                    Command::new("average")
                        .about("Get the average value of the stock for the given period (length) and interval")
                )
                .subcommand(
                    Command::new("volume")
                        .about("Get the volume of the stock for the given period (length) and interval")
                )
                .subcommand(
                    Command::new("last")
                        .about("Get the last value of the stock. Sets the `length` to 1 day and `interval` to 0")
                )
        )
        .subcommand(
            Command::new("export")
                .about("Export data from your BoursoBank account")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("transactions")
                        .about("Export transactions for a given account over a date range")
                        .arg(account_arg.clone())
                        .arg(
                            Arg::new("start-date")
                                .long("start-date")
                                .help("Start date for transactions (DD/MM/YYYY)")
                                .required(true)
                        )
                        .arg(
                            Arg::new("end-date")
                                .long("end-date")
                                .help("End date for transactions (DD/MM/YYYY)")
                                .required(true)
                        )
                        .arg(
                            Arg::new("format")
                                .long("format")
                                .short('f')
                                .help("Output format (csv or json)")
                                .default_value("csv")
                                .value_parser(["csv", "json"])
                        )
                        .arg(
                            Arg::new("output")
                                .long("output")
                                .short('o')
                                .help("Output file path (defaults to stdout)")
                                .required(false)
                        )
                )
        )
        .subcommand(bourso_cli::money::command(account_arg.clone()))
        .arg(
            Arg::new("credentials")
                .long("credentials")
                .help("The path to the credentials file")
                .value_parser(clap::value_parser!(String))
                .required(false)
        )
        .try_get_matches_from(args);
    let matches = match matches {
        Ok(matches) => matches,
        Err(error) => {
            if sensitive
                && !matches!(
                    error.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                )
            {
                bourso_cli::financial_guard::Report::not_emitted().print();
                eprintln!("Check command options (including --amount for transfers); values omitted for privacy.");
                std::process::exit(2);
            }
            error.exit();
        }
    };

    let financial = bourso_cli::financial_guard::is_financial(&matches);
    // Financial commands intentionally install no tracing subscriber: upstream
    // diagnostics can carry identifiers or raw provider responses at any level.
    let result = bourso_cli::parse_matches(matches).await;
    if financial {
        if let Err(error) = result {
            let fallback = bourso_cli::financial_guard::Report::not_emitted();
            let report = error
                .downcast_ref::<bourso_cli::financial_guard::Report>()
                .unwrap_or(&fallback);
            report.print();
            std::process::exit(report.exit_code().into());
        }
    } else {
        result?;
    }

    Ok(())
}
