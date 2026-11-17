//! The errors a business rule can return.
//!
//! A subset of the server's `Error`, without the ones that only the web, the database or an integration can
//! raise. The server converts each into its own variant of the same name, so `?` works across the two crates
//! and the HTTP status a rule's refusal maps to is unchanged.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("{0}")]
    Unprocessable(String),

    #[error("Invalid state transition: {0}")]
    InvalidStateTransition(String),

    #[error("Conflict: {0}")]
    Conflict(String),
}

impl Error {
    pub fn not_found(msg: impl Into<String>) -> Self {
        Error::NotFound(msg.into())
    }

    pub fn validation(msg: impl Into<String>) -> Self {
        Error::Validation(msg.into())
    }

    pub fn unprocessable(msg: impl Into<String>) -> Self {
        Error::Unprocessable(msg.into())
    }

    pub fn invalid_transition(msg: impl Into<String>) -> Self {
        Error::InvalidStateTransition(msg.into())
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Error::Conflict(msg.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_match_the_server_wording() {
        assert_eq!(Error::validation("x").to_string(), "Validation error: x");
        assert_eq!(
            Error::invalid_transition("a to b").to_string(),
            "Invalid state transition: a to b"
        );
        assert_eq!(Error::unprocessable("plain").to_string(), "plain");
    }
}
