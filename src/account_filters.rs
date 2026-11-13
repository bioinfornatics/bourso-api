//! Pure command parsing for account categories; no logging, configuration or network effects.
use clap::{Arg, ArgAction, ArgGroup, ArgMatches, Command};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountFilter {
    Banking,
    Savings,
    Trading,
    Loans,
}

/// A category is optional, but categories are mutually exclusive.
pub fn command() -> Command {
    Command::new("accounts")
        .about("List your accounts, optionally filtered by one category")
        .arg(
            Arg::new("bank")
                .long("banking")
                .help("List banking accounts")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("saving")
                .long("saving")
                .help("List savings accounts")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("trading")
                .long("trading")
                .help("List trading accounts")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("loans")
                .long("loans")
                .help("List loan accounts")
                .action(ArgAction::SetTrue),
        )
        .group(
            ArgGroup::new("account_category")
                .args(["bank", "saving", "trading", "loans"])
                .multiple(false),
        )
}

/// Read boolean values, not argument presence: absent SetTrue flags have a false default.
pub fn selected(matches: &ArgMatches) -> Option<AccountFilter> {
    [
        ("bank", AccountFilter::Banking),
        ("saving", AccountFilter::Savings),
        ("trading", AccountFilter::Trading),
        ("loans", AccountFilter::Loans),
    ]
    .into_iter()
    .find_map(|(id, category)| matches.get_flag(id).then_some(category))
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn each_flag_selects_its_category_without_a_value() {
        for (flag, expected) in [
            ("--banking", AccountFilter::Banking),
            ("--saving", AccountFilter::Savings),
            ("--trading", AccountFilter::Trading),
            ("--loans", AccountFilter::Loans),
        ] {
            let matches = command().try_get_matches_from(["accounts", flag]).unwrap();
            assert_eq!(selected(&matches), Some(expected));
        }
    }

    #[test]
    fn no_flag_lists_all_categories() {
        let matches = command().try_get_matches_from(["accounts"]).unwrap();
        assert_eq!(selected(&matches), None);
    }

    #[test]
    fn every_pair_of_categories_conflicts() {
        let flags = ["--banking", "--saving", "--trading", "--loans"];
        for left in flags {
            for right in flags {
                if left != right {
                    let error = command()
                        .try_get_matches_from(["accounts", left, right])
                        .unwrap_err();
                    assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
                }
            }
        }
    }

    #[test]
    fn flags_reject_unexpected_values() {
        for flag in ["--banking", "--saving", "--trading", "--loans"] {
            let error = command()
                .try_get_matches_from(["accounts", flag, "arbitrary"])
                .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::UnknownArgument);
            assert!(command()
                .try_get_matches_from(["accounts", &format!("{flag}=true")])
                .is_err());
        }
    }

    #[test]
    fn command_definition_is_consistent() {
        command().debug_assert();
    }
}
