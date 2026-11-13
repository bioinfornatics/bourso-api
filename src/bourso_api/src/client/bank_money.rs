//! Checked parsing of bank display/export amounts (not transfer-entry policy).
//!
//! Integers denote euros. A dot or comma introduces one or two fractional
//! digits. Grouping is optional, using ASCII space, NBSP or narrow NBSP, with
//! three digits per group. One leading ASCII sign or Unicode minus is accepted.
//! No exponents, currency symbols, non-finite values, or rounding are permitted.
use anyhow::{bail, Context, Result};

pub(crate) fn parse_cents(input: &str) -> Result<i64> {
    let normalized = input.trim().replace(['\u{a0}', '\u{202f}'], " ");
    let (negative, number) = if let Some(number) = normalized.strip_prefix(['-', '−']) {
        (true, number.trim_start_matches(' '))
    } else if let Some(number) = normalized.strip_prefix('+') {
        (false, number.trim_start_matches(' '))
    } else {
        (false, normalized.as_str())
    };
    let (whole, fraction) = match number.split_once(['.', ',']) {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (number, None),
    };
    let groups: Vec<_> = whole.split(' ').collect();
    if groups
        .iter()
        .any(|group| group.is_empty() || !group.bytes().all(|digit| digit.is_ascii_digit()))
        || (groups.len() > 1
            && (groups[0].len() > 3 || groups[1..].iter().any(|group| group.len() != 3)))
    {
        bail!("Invalid bank amount syntax");
    }
    let fractional_cents = match fraction {
        None => 0,
        Some(part)
            if (1..=2).contains(&part.len())
                && part.bytes().all(|digit| digit.is_ascii_digit()) =>
        {
            i64::from(part.as_bytes()[0] - b'0') * 10
                + if part.len() == 2 {
                    i64::from(part.as_bytes()[1] - b'0')
                } else {
                    0
                }
        }
        Some(_) => bail!("Invalid bank amount precision or separator"),
    };
    // Accumulate negative values to include i64::MIN without an abs overflow.
    let euros = whole
        .bytes()
        .filter(|digit| *digit != b' ')
        .try_fold(0i64, |value, digit| {
            value
                .checked_mul(10)
                .and_then(|value| value.checked_sub(i64::from(digit - b'0')))
                .context("Bank amount exceeds supported cent range")
        })?;
    let cents = euros
        .checked_mul(100)
        .and_then(|value| value.checked_sub(fractional_cents))
        .context("Bank amount exceeds supported cent range")?;
    if negative {
        Ok(cents)
    } else {
        cents
            .checked_neg()
            .context("Bank amount exceeds supported cent range")
    }
}

/// Compatibility boundary for Transaction's public f64 euro fields.
/// This conservative limit keeps adjacent cents distinct and cent round trips
/// reliable despite binary fractional rounding. Parsing itself is integer-only.
pub(crate) fn cents_to_euros(cents: i64) -> Result<f64> {
    const MAX_CENTS: i64 = (1i64 << 51) - 1;
    if !(-MAX_CENTS..=MAX_CENTS).contains(&cents) {
        bail!("Bank amount exceeds cent-preserving transaction range");
    }
    Ok(cents as f64 / 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exact_signed_cents() {
        for (input, expected) in [
            ("12", 1200),
            ("12,3", 1230),
            ("12.30", 1230),
            ("0", 0),
            ("-0,01", -1),
            ("+12", 1200),
            ("-0", 0),
            ("1 234,56", 123456),
            ("1\u{a0}234,56", 123456),
            ("−\u{202f}1\u{202f}234,56", -123456),
            (" \t- 12.30\n", -1230),
            ("92233720368547758.07", i64::MAX),
            ("-92233720368547758.08", i64::MIN),
        ] {
            assert_eq!(parse_cents(input).unwrap(), expected);
        }
    }

    #[test]
    fn rejects_malformed_and_overflow_without_echoing_input() {
        for input in [
            "",
            " ",
            "NaN",
            "inf",
            "-inf",
            "1e2",
            "12,345",
            "12.000",
            "12,",
            ".12",
            "1,2.3",
            "1.234,56",
            "1 23",
            "1234 567",
            "1  234",
            "1\t234",
            "1\n234",
            "12 ,30",
            "12, 30",
            "--12",
            "−-12",
            "12-",
            "１２",
            "12€",
            "private-invalid-value",
            "92233720368547758.08",
            "-92233720368547758.09",
            "999999999999999999999999999999999999999999999",
        ] {
            assert!(parse_cents(input).is_err());
        }
        let error = parse_cents("private-invalid-value").unwrap_err();
        assert!(!format!("{error:?}").contains("private-invalid-value"));
    }

    #[test]
    fn transaction_boundary_preserves_cents() {
        let max = (1i64 << 51) - 1;
        for cents in [-max, -123456, -1, 0, 1, 29, 123456, max] {
            let euros = cents_to_euros(cents).unwrap();
            assert!(euros.is_finite());
            assert_eq!((euros * 100.0).round() as i64, cents);
            assert_eq!(parse_cents(&euros.to_string()).unwrap(), cents);
        }
        assert!(cents_to_euros(max + 1).is_err());
        assert!(cents_to_euros(-max - 1).is_err());
        assert!(cents_to_euros(i64::MIN).is_err());
        assert!(cents_to_euros(i64::MAX).is_err());
    }
}
