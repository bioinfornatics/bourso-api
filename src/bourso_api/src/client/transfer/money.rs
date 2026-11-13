//! Exact transfer amounts, shared with the CLI.
//!
//! Policy: unsigned ASCII decimal euros, an optional dot or comma followed by
//! one or two digits, no whitespace/sign/exponent/grouping. The existing minimum
//! is 10 EUR. The maximum is u64::MAX cents (184467440737095516.15 EUR).
//! Excess precision is rejected, never rounded, even for trailing zeroes.
use std::{fmt, str::FromStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Money(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoneyError {
    InvalidDecimal,
    TooLow,
    Overflow,
}

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidDecimal => "Invalid amount: use unsigned decimal euros with at most two fractional digits (dot or comma)",
            Self::TooLow => "Amount is below the minimum threshold (10 EUR)",
            Self::Overflow => "Amount exceeds the supported cent range",
        })
    }
}
impl std::error::Error for MoneyError {}

impl FromStr for Money {
    type Err = MoneyError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let (whole, fraction) = match input.split_once(['.', ',']) {
            Some((whole, fraction)) => (whole, Some(fraction)),
            None => (input, None),
        };
        if whole.is_empty() || !whole.bytes().all(|digit| digit.is_ascii_digit()) {
            return Err(MoneyError::InvalidDecimal);
        }
        let fractional_cents = match fraction {
            Some(part)
                if (1..=2).contains(&part.len())
                    && part.bytes().all(|digit| digit.is_ascii_digit()) =>
            {
                let first = u64::from(part.as_bytes()[0] - b'0');
                first * 10
                    + if part.len() == 2 {
                        u64::from(part.as_bytes()[1] - b'0')
                    } else {
                        0
                    }
            }
            Some(_) => return Err(MoneyError::InvalidDecimal),
            None => 0,
        };
        let euros = whole.bytes().try_fold(0u64, |value, digit| {
            value
                .checked_mul(10)
                .and_then(|value| value.checked_add(u64::from(digit - b'0')))
                .ok_or(MoneyError::Overflow)
        })?;
        let cents = euros
            .checked_mul(100)
            .and_then(|value| value.checked_add(fractional_cents))
            .ok_or(MoneyError::Overflow)?;
        if cents < 1_000 {
            return Err(MoneyError::TooLow);
        }
        Ok(Self(cents))
    }
}

impl TryFrom<f64> for Money {
    type Error = MoneyError;

    /// Compatibility boundary: validate the shortest round-trip decimal spelling.
    /// Non-finite/negative numbers and sub-cent values are never rounded or given
    /// a tolerance. Callers needing exact decimal input should use FromStr.
    fn try_from(amount: f64) -> Result<Self, Self::Error> {
        if !amount.is_finite() || amount.is_sign_negative() {
            return Err(MoneyError::InvalidDecimal);
        }
        amount.to_string().parse()
    }
}

impl Money {
    pub fn cents(self) -> u64 {
        self.0
    }

    /// Bank form spelling, derived only from validated integer cents.
    pub fn bank_amount(self) -> String {
        format!("{},{:02}", self.0 / 100, self.0 % 100)
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:02}", self.0 / 100, self.0 % 100)
    }
}
