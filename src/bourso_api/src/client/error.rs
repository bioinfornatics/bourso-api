use std::fmt;

pub enum ClientError {
    InvalidCredentials,
    MfaRequired,
    QRCodeRequired(String),
    InvalidMfa,
}

impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ClientError::InvalidCredentials => write!(f, "Invalid credentials"),
            ClientError::MfaRequired => write!(f, "MFA required"),
            ClientError::QRCodeRequired(_) => write!(f, "QR code authentication required"),
            ClientError::InvalidMfa => write!(f, "Invalid MFA"),
        }
    }
}

// QR data remains available to the MFA consumer, never to diagnostic formatting.
impl fmt::Debug for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCredentials => f.write_str("InvalidCredentials"),
            Self::MfaRequired => f.write_str("MfaRequired"),
            Self::QRCodeRequired(_) => f.write_str("QRCodeRequired([redacted])"),
            Self::InvalidMfa => f.write_str("InvalidMfa"),
        }
    }
}
