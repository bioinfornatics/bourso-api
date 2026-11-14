use clap::{Arg, Command};

pub use bourso_api::client::transfer::money::{Money, MoneyError};

/// Validate while retaining the historical String-valued ArgMatches contract.
pub fn validate_amount(input: &str) -> Result<String, MoneyError> {
    input.parse::<Money>()?;
    Ok(input.to_owned())
}

/// Build the transfer command; shared by the binary and parser tests.
pub fn command(account_arg: Arg) -> Command {
    Command::new("transfer")
        .about("Make a transfer between your accounts")
        .arg(account_arg.clone())
        .arg(
            Arg::new("to_account")
                .long("to")
                .help(
                    r#"The destination account id as an hexadecimal string (32 characters).
    You can get this info with the command `bourso accounts`"#,
                )
                .required(true),
        )
        .arg(
            Arg::new("amount")
                .long("amount")
                .help("The amount to transfer (minimum 10 EUR, at most two decimals; dot or comma)")
                .value_parser(validate_amount)
                .required(true),
        )
        .arg(
            Arg::new("reason")
                .long("reason")
                .help("The reason for the transfer (local max 50 UTF-8 bytes; provider contract unverified)")
                .required(false),
        )
}
