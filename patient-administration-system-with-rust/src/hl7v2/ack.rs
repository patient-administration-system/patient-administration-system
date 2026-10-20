//! HL7 v2 ACK builder.
//!
//! Returns the standard `MSH|^~\&|...|ACK|...\rMSA|AA|<control_id>` envelope.

use chrono::Utc;

use super::{encode_message, message, segment};

/// MSA-1 acknowledgement code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckCode {
    /// `AA` — Application Accept.
    Accept,
    /// `AE` — Application Error (the message was well-formed but rejected).
    AppError,
    /// `AR` — Application Reject (the message was rejected before processing).
    Reject,
}

impl AckCode {
    pub fn as_str(self) -> &'static str {
        match self {
            AckCode::Accept => "AA",
            AckCode::AppError => "AE",
            AckCode::Reject => "AR",
        }
    }
}

/// Build an ACK for a received message, referring back to its
/// `message_control_id` (MSH-10).
pub fn ack(
    sending_app: &str,
    receiving_app: &str,
    message_control_id: &str,
    code: AckCode,
    diagnostics: Option<&str>,
) -> String {
    let now = Utc::now().format("%Y%m%d%H%M%S").to_string();
    let msh = segment(
        "MSH",
        vec![
            "|".into(),
            "^~\\&".into(),
            sending_app.into(),
            "FAC".into(),
            receiving_app.into(),
            "FAC".into(),
            now,
            String::new(),
            "ACK".into(),
            message_control_id.into(),
            "P".into(),
            "2.5".into(),
        ],
    );
    let mut msa_fields = vec![code.as_str().into(), message_control_id.into()];
    if let Some(d) = diagnostics {
        msa_fields.push(d.into());
    }
    let msa = segment("MSA", msa_fields);
    encode_message(&message(vec![msh, msa]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ack_accept_shape() {
        let s = ack("PAS", "EMR", "MSG001", AckCode::Accept, None);
        assert!(s.starts_with("MSH|^~\\&|PAS|FAC|EMR|FAC|"));
        assert!(s.contains("|ACK|MSG001|P|2.5\r"));
        assert!(s.contains("MSA|AA|MSG001\r"));
    }

    #[test]
    fn test_ack_app_error_includes_diagnostics() {
        let s = ack(
            "PAS",
            "EMR",
            "MSG002",
            AckCode::AppError,
            Some("PID-5 missing"),
        );
        assert!(s.contains("MSA|AE|MSG002|PID-5 missing\r"));
    }
}
