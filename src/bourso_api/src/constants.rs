pub const BASE_URL: &str = "https://clients.boursobank.com";
// Tokenize complete HTML tags (including quoted >) and comments. Account parsing
// balances nested lists explicitly rather than treating the first close as root.
pub const HTML_TAG_PATTERN: &str =
    r#"(?s)<!--.*?-->|</?[a-zA-Z][a-zA-Z0-9:_-]*(?:[^>"']|"[^"]*"|'[^']*')*>"#;
// Complete attribute tokens; quoted values cannot introduce summary markers.
pub const ACCOUNT_SECTION_PATTERN: &str = r#"([^\s=<>/]+)(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s>]+))?"#;
// Capture the complete amount, including malformed text, for checked parsing.
// A match is confined to one balanced account row, never its sibling.
pub const ACCOUNT_PATTERN: &str = r#"(?s)href=["']/(?:budget/)?compte/(?:[^/"'<>]+/)*?(?P<id>[a-f0-9]{32})/["'][^<>]*Solde\s:\s(?P<balance>[^"<>€]*)€.*?c-info-box__account-label[^<>]*>(?P<name>[^<>]*)</span>.*?c-info-box__account-sub-label[^<>]*>(?P<bank_name>[^<>]*)</span>"#;
