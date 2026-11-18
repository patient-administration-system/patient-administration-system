//! The TLS crypto provider. reqwest is built with `rustls-no-provider` so that only ring is compiled (not
//! aws-lc-rs); rustls then needs a process-wide provider installed before the first client is built, or reqwest
//! panics with "No rustls crypto provider is configured". Every place that builds an HTTP client calls
//! [`ensure_provider`], and so does `main`, so tests and binaries both get it.

use std::sync::Once;

/// Install ring as the process's rustls provider, once. Safe to call from anywhere, any number of times; if
/// another provider was installed first, that one stays.
pub fn ensure_provider() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_client_builds_once_the_provider_is_installed() {
        super::ensure_provider();
        super::ensure_provider();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
        assert!(reqwest::Client::builder().build().is_ok());
    }
}
