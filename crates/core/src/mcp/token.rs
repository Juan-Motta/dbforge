//! The bearer token agents send: random, created on first use, and kept in
//! the credential store with the connections' passwords.

use crate::credentials::Credentials;
use crate::error::Result;

/// Its key in the credential store; connection ids are UUIDs, so it never clashes with one.
pub const CREDENTIAL_ID: &str = "dbforge-mcp-token";

/// The token, made the first time it is asked for.
pub fn get_or_create(credentials: &Credentials) -> Result<String> {
    match credentials.get(CREDENTIAL_ID)? {
        Some(token) => Ok(token),
        None => regenerate(credentials),
    }
}

/// A new token in place of the old one, which stops working.
pub fn regenerate(credentials: &Credentials) -> Result<String> {
    let token = new_token();
    credentials.set(CREDENTIAL_ID, &token)?;
    Ok(token)
}

/// 244 random bits from the OS generator (two v4 UUIDs), as 64 hex digits.
fn new_token() -> String {
    format!("{}{}", uuid::Uuid::new_v4().simple(), uuid::Uuid::new_v4().simple())
}

/// Compares in time that depends only on the lengths, so a caller can't guess the token byte by byte.
pub fn matches(expected: &str, given: &str) -> bool {
    let (expected, given) = (expected.as_bytes(), given.as_bytes());
    expected.len() == given.len() && expected.iter().zip(given).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_long_hex_and_differ() {
        let (a, b) = (new_token(), new_token());
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn matches_only_the_exact_token() {
        assert!(matches("abc123", "abc123"));
        assert!(!matches("abc123", "abc124"));
        assert!(!matches("abc123", "abc12"));
        assert!(!matches("abc123", ""));
    }
}
