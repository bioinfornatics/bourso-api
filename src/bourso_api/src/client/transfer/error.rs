use std::fmt;

#[derive(Debug)]
pub enum TransferError {
    AmountTooLow,
    TransferInitiationFailed,
    SetDebitAccountFailed,
    SetCreditAccountFailed,
    Step5Failed,
    SetAmountFailed,
    SetReasonFailed,
    ReasonIsTooLong,
}

impl fmt::Display for TransferError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TransferError::AmountTooLow => {
                write!(f, "Amount is below the minimum threshold (10 EUR)")
            }
            TransferError::ReasonIsTooLong => {
                write!(f, "Transfer reason is too long, max 50 UTF-8 bytes, local policy; provider contract unverified")
            }
            TransferError::TransferInitiationFailed => write!(f, "Transfer initiation failed"),
            TransferError::SetDebitAccountFailed => write!(f, "Setting debit account failed"),
            TransferError::SetCreditAccountFailed => write!(f, "Setting credit account failed"),
            TransferError::Step5Failed => write!(f, "Transfer step 5 failed"),
            TransferError::SetAmountFailed => write!(f, "Setting transfer amount failed"),
            TransferError::SetReasonFailed => write!(f, "Setting transfer reason failed"),
        }
    }
}

impl std::error::Error for TransferError {}
