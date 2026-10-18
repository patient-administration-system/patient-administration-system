//! Per-channel message senders (email, SMS, Apple Messages, WhatsApp).
//!
//! The PAS does not talk to carriers directly. Each channel is a
//! [`MessageSender`]; the default for every channel is disabled, so nothing is
//! ever sent until a deployment wires a real sender (or `PAS_<CHANNEL>_PROVIDER=log`).

use async_trait::async_trait;
use std::sync::Arc;

use super::sms::SmsProvider;
use crate::Result;

#[async_trait]
pub trait MessageSender: Send + Sync {
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<()>;

    fn is_enabled(&self) -> bool {
        true
    }
}

/// Logs the message instead of sending it. Useful for development.
pub struct LogMessageSender {
    pub channel: &'static str,
}

#[async_trait]
impl MessageSender for LogMessageSender {
    async fn send(&self, to: &str, subject: &str, body: &str) -> Result<()> {
        tracing::info!(
            target: "pas::message",
            channel = self.channel,
            recipient = %to,
            "message dispatched (log-only): {subject}: {body}"
        );
        Ok(())
    }
}

/// Adapts the existing [`SmsProvider`] to the generic sender interface.
pub struct SmsSender(pub Arc<dyn SmsProvider>);

#[async_trait]
impl MessageSender for SmsSender {
    async fn send(&self, to: &str, _subject: &str, body: &str) -> Result<()> {
        self.0.send(to, body).await
    }

    fn is_enabled(&self) -> bool {
        self.0.is_enabled()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::communication::{LogSmsProvider, NoopSmsProvider};

    #[tokio::test]
    async fn test_log_sender_is_enabled() {
        let s = LogMessageSender {
            channel: "whatsapp",
        };
        assert!(s.is_enabled());
        assert!(s.send("+15555550100", "s", "b").await.is_ok());
    }

    #[tokio::test]
    async fn test_sms_sender_mirrors_provider_enabled_flag() {
        assert!(SmsSender(Arc::new(LogSmsProvider)).is_enabled());
        assert!(!SmsSender(Arc::new(NoopSmsProvider)).is_enabled());
    }
}
