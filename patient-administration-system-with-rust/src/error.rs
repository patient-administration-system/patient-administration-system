//! Error types for the Patient Administration System

use thiserror::Error;

/// Result type alias for PAS operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for the Patient Administration System.
#[derive(Error, Debug)]
pub enum Error {
    #[error("Database error: {0}")]
    Database(#[from] sea_orm::DbErr),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Invalid state transition: {0}")]
    InvalidStateTransition(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    /// The caller may not do this: a consent it needs is not there (DTAC-7). Maps to 403.
    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Search error: {0}")]
    Search(String),

    #[error("Streaming error: {0}")]
    Streaming(String),

    #[error("FHIR error: {0}")]
    Fhir(String),

    #[error("Render error: {0}")]
    Render(String),

    #[error("Configuration error: {0}")]
    Config(String),

    /// An upstream service (`person-service` / `worker-service`) was
    /// unreachable, returned a non-translatable non-2xx, or otherwise failed
    /// in a way the PAS cannot recover from per-request. Maps to
    /// `503 + UPSTREAM_UNAVAILABLE`. Translatable 4xx envelopes from upstream
    /// are surfaced as the matching variant (`NotFound`, `Validation`,
    /// `Conflict`) instead — see `src/upstream/`.
    #[error("Upstream service error: {0}")]
    Upstream(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

impl Error {
    /// Build a `Forbidden` error.
    pub fn forbidden(msg: impl Into<String>) -> Self {
        Error::Forbidden(msg.into())
    }

    /// Build a `NotFound` error.
    pub fn not_found(msg: impl Into<String>) -> Self {
        Error::NotFound(msg.into())
    }

    /// Build a `Validation` error.
    pub fn validation(msg: impl Into<String>) -> Self {
        Error::Validation(msg.into())
    }

    /// Build an `InvalidStateTransition` error.
    pub fn invalid_transition(msg: impl Into<String>) -> Self {
        Error::InvalidStateTransition(msg.into())
    }

    /// Build a `Conflict` error.
    pub fn conflict(msg: impl Into<String>) -> Self {
        Error::Conflict(msg.into())
    }

    /// Build an `Internal` error.
    pub fn internal(msg: impl Into<String>) -> Self {
        Error::Internal(msg.into())
    }

    /// Build a `Config` error.
    pub fn config(msg: impl Into<String>) -> Self {
        Error::Config(msg.into())
    }

    /// Build a `Render` error.
    pub fn render(msg: impl Into<String>) -> Self {
        Error::Render(msg.into())
    }

    /// Build an `Upstream` error.
    pub fn upstream(msg: impl Into<String>) -> Self {
        Error::Upstream(msg.into())
    }

    /// The text that may leave the process in an API or HL7 reply.
    ///
    /// Errors whose text is written for the caller (not found, validation,
    /// conflict, forbidden, FHIR, state transition) pass through. The rest
    /// carry text from libraries and other services (SQL, constraint and
    /// table names, URLs): those are logged in full with a short reference
    /// and the caller gets generic text and the reference.
    pub fn client_message(&self) -> String {
        let what = match self {
            Error::NotFound(_)
            | Error::Validation(_)
            | Error::InvalidStateTransition(_)
            | Error::Conflict(_)
            | Error::Forbidden(_)
            | Error::Fhir(_) => return self.to_string(),
            Error::Database(_) => "Database error",
            Error::Search(_) => "Search error",
            Error::Streaming(_) => "Streaming error",
            Error::Render(_) => "Render error",
            Error::Config(_) => "Configuration error",
            Error::Upstream(_) => "Upstream service error",
            Error::Internal(_) => "Internal error",
        };
        let reference = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        tracing::error!(%reference, error = %self, "error text withheld from the caller");
        format!("{what} (reference {reference})")
    }
}

impl From<sea_orm::TransactionError<Error>> for Error {
    /// A failed transaction is either the connection (a database error) or the error the closure returned.
    fn from(e: sea_orm::TransactionError<Error>) -> Self {
        match e {
            sea_orm::TransactionError::Connection(c) => Error::Database(c),
            sea_orm::TransactionError::Transaction(t) => t,
        }
    }
}

/// [`Error::client_message`] for any error a handler might hold: a
/// `crate::Error` is sanitised, any other error (such as an HL7 parse
/// error, which only describes the caller's own message) is shown as is.
pub fn client_text<E: std::fmt::Display + 'static>(e: &E) -> String {
    match (e as &dyn std::any::Any).downcast_ref::<Error>() {
        Some(err) => err.client_message(),
        None => e.to_string(),
    }
}

#[cfg(test)]
mod client_message_tests {
    use super::*;

    #[test]
    fn database_text_never_reaches_the_caller() {
        let e = Error::Database(sea_orm::DbErr::Custom(
            "relation \"patients\" does not exist; constraint uq_x".into(),
        ));
        let text = e.client_message();
        assert!(!text.contains("relation") && !text.contains("patients") && !text.contains("uq_x"));
        assert!(text.starts_with("Database error (reference "), "{text}");
    }

    #[test]
    fn internal_config_and_upstream_text_is_withheld() {
        for e in [
            Error::internal("secret path /srv/x"),
            Error::config("PAS_SECRET missing"),
            Error::upstream("http://person-service:5180/api/persons failed"),
        ] {
            let text = e.client_message();
            assert!(
                !text.contains("/srv") && !text.contains("PAS_SECRET") && !text.contains("5180"),
                "{text}"
            );
        }
    }

    #[test]
    fn messages_written_for_the_caller_pass_through() {
        assert_eq!(
            Error::validation("name is required").client_message(),
            "Validation error: name is required"
        );
        assert_eq!(
            Error::not_found("appointment").client_message(),
            "Not found: appointment"
        );
    }

    #[test]
    fn client_text_sanitises_a_crate_error_and_shows_other_errors() {
        let e = Error::Database(sea_orm::DbErr::Custom("relation \"x\"".into()));
        assert!(!client_text(&e).contains("relation"));
        assert_eq!(client_text(&"bad segment"), "bad segment".to_string());
    }
}
