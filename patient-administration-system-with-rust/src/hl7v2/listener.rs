//! TCP listener that accepts MLLP-framed HL7 v2 messages and dispatches
//! them to the existing HTTP handlers via `tower::ServiceExt::oneshot`.
//!
//! Design: zero handler refactoring. The listener inspects the message
//! type, picks the right `/api/hl7/v2/...` endpoint, builds a synthetic
//! request, and hands it to the router. The response body is the ACK
//! envelope, which the listener writes back in MLLP framing.

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tower::ServiceExt;

use crate::hl7v2::mllp::Frame;

use crate::hl7v2::{Message, SegmentRead, mllp, parse_message};
use crate::{Error, Result};

/// Local replacement for the deleted `mapping::message_type` helper.
/// Reads MSH-9.1 (message code) and MSH-9.2 (trigger event); returns
/// empty strings when either is missing.
fn message_type(msg: &Message) -> (String, String) {
    let msh = match msg.segment("MSH") {
        Some(s) => s,
        None => return (String::new(), String::new()),
    };
    let field = msh.raw(9);
    let mut parts = field.split('^');
    let code = parts.next().unwrap_or("").to_string();
    let event = parts.next().unwrap_or("").to_string();
    (code, event)
}

/// Whether this process's MLLP listener is taking connections, for `GET /api/health` (NFR-34).
static LISTENER_STATE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(OFF);
const OFF: u8 = 0;
const ACCEPTING: u8 = 1;
const STOPPED: u8 = 2;

/// `off` (no listener in this process), `accepting`, or `stopped` (it was started and is no longer taking
/// connections: the bind or an accept failed).
pub fn listener_state() -> &'static str {
    match LISTENER_STATE.load(std::sync::atomic::Ordering::Relaxed) {
        ACCEPTING => "accepting",
        STOPPED => "stopped",
        _ => "off",
    }
}

fn set_state(state: u8) {
    LISTENER_STATE.store(state, std::sync::atomic::Ordering::Relaxed);
}

/// How the MLLP listener takes connections (NFR-31, HUB-3).
#[derive(Clone)]
pub struct MllpOptions {
    /// TLS, when a certificate and key are configured; plain TCP otherwise.
    pub tls: Option<Arc<rustls::ServerConfig>>,
    /// A connection with no frame for this long is closed (`PAS_HL7_IDLE_SECONDS`, default 30).
    pub idle: Duration,
    /// Connections open at once; one more is closed at once (`PAS_HL7_MAX_CONNECTIONS`, default 64).
    pub max_connections: usize,
}

impl Default for MllpOptions {
    fn default() -> Self {
        Self {
            tls: None,
            idle: Duration::from_secs(30),
            max_connections: 64,
        }
    }
}

impl MllpOptions {
    /// From the environment. TLS needs both `PAS_HL7_TLS_CERT_FILE` and `PAS_HL7_TLS_KEY_FILE`; one without the
    /// other is an error, so the server refuses to start rather than listen in plain text by mistake.
    /// `PAS_HL7_TLS_CLIENT_CA_FILE` also asks each sender for a certificate signed by that CA.
    pub fn from_env() -> Result<Self> {
        use crate::config::env;
        Self::from_settings(
            env::text("PAS_HL7_TLS_CERT_FILE"),
            env::text("PAS_HL7_TLS_KEY_FILE"),
            env::text("PAS_HL7_TLS_CLIENT_CA_FILE"),
            Duration::from_secs(u64::try_from(env::int("PAS_HL7_IDLE_SECONDS")).unwrap_or(30)),
            usize::try_from(env::int("PAS_HL7_MAX_CONNECTIONS")).unwrap_or(64),
        )
    }

    /// [`Self::from_env`] from the values themselves: file paths for the certificate, key and client CA.
    pub fn from_settings(
        cert: Option<String>,
        key: Option<String>,
        client_ca: Option<String>,
        idle: Duration,
        max_connections: usize,
    ) -> Result<Self> {
        let read = |name: &str, path: &str| {
            std::fs::read(path).map_err(|e| Error::config(format!("{name}={path:?}: {e}")))
        };
        let tls = match (cert, key) {
            (None, None) => {
                if client_ca.is_some() {
                    return Err(Error::config(
                        "PAS_HL7_TLS_CLIENT_CA_FILE needs PAS_HL7_TLS_CERT_FILE and PAS_HL7_TLS_KEY_FILE",
                    ));
                }
                None
            }
            (Some(cert), Some(key)) => {
                let ca = match &client_ca {
                    Some(path) => Some(read("PAS_HL7_TLS_CLIENT_CA_FILE", path)?),
                    None => None,
                };
                Some(Arc::new(tls_server_config(
                    &read("PAS_HL7_TLS_CERT_FILE", &cert)?,
                    &read("PAS_HL7_TLS_KEY_FILE", &key)?,
                    ca.as_deref(),
                )?))
            }
            _ => {
                return Err(Error::config(
                    "PAS_HL7_TLS_CERT_FILE and PAS_HL7_TLS_KEY_FILE must be set together",
                ));
            }
        };
        Ok(Self {
            tls,
            idle,
            max_connections,
        })
    }
}

/// A rustls server configuration from PEM: the certificate chain, its private key, and optionally the CA a
/// sender's certificate must chain to. The ring provider, as sqlx uses.
pub fn tls_server_config(
    cert_pem: &[u8],
    key_pem: &[u8],
    client_ca_pem: Option<&[u8]>,
) -> Result<rustls::ServerConfig> {
    use rustls_pki_types::pem::PemObject;
    use rustls_pki_types::{CertificateDer, PrivateKeyDer};
    let bad =
        |what: &str, e: &dyn std::fmt::Display| Error::config(format!("MLLP TLS {what}: {e}"));
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| bad("certificate", &e))?;
    if certs.is_empty() {
        return Err(Error::config(
            "MLLP TLS certificate: no certificate in the file",
        ));
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem).map_err(|e| bad("key", &e))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = rustls::ServerConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| bad("protocol versions", &e))?;
    let builder = match client_ca_pem {
        None => builder.with_no_client_auth(),
        Some(ca_pem) => {
            let mut roots = rustls::RootCertStore::empty();
            for ca in CertificateDer::pem_slice_iter(ca_pem) {
                roots
                    .add(ca.map_err(|e| bad("client CA", &e))?)
                    .map_err(|e| bad("client CA", &e))?;
            }
            let verifier = rustls::server::WebPkiClientVerifier::builder_with_provider(
                Arc::new(roots),
                provider,
            )
            .build()
            .map_err(|e| bad("client CA", &e))?;
            builder.with_client_cert_verifier(verifier)
        }
    };
    builder
        .with_single_cert(certs, key)
        .map_err(|e| bad("certificate and key", &e))
}

/// MLLP TCP server. Owns a clone of the axum router and accepts connections
/// in a loop; each connection runs on its own tokio task and may carry
/// multiple frames.
pub struct MllpServer {
    router: Router,
    options: MllpOptions,
}

impl MllpServer {
    pub fn new(router: Router) -> Self {
        Self {
            router,
            options: MllpOptions::default(),
        }
    }

    /// The same server with `options` (TLS, idle timeout, connection cap).
    pub fn with_options(mut self, options: MllpOptions) -> Self {
        self.options = options;
        self
    }

    /// Bind and accept connections forever. Returns only on a fatal accept
    /// error (e.g. socket closed); per-connection errors are logged and the
    /// connection is dropped, but the listener keeps running.
    pub async fn serve(self, bind: &str) -> Result<()> {
        let listener = TcpListener::bind(bind).await.map_err(|e| {
            set_state(STOPPED);
            Error::Streaming(format!("MLLP bind {bind}: {e}"))
        })?;
        tracing::info!(target: "pas::hl7v2::mllp", tls = self.options.tls.is_some(), "MLLP listener bound on {bind}");
        self.serve_listener(listener).await
    }

    /// Accept on a listener already bound (the tests bind port 0).
    pub async fn serve_listener(self, listener: TcpListener) -> Result<()> {
        set_state(ACCEPTING);
        let slots = Arc::new(tokio::sync::Semaphore::new(
            self.options.max_connections.max(1),
        ));
        let acceptor = self
            .options
            .tls
            .clone()
            .map(tokio_rustls::TlsAcceptor::from);
        let idle = self.options.idle;
        loop {
            let (sock, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    set_state(STOPPED);
                    tracing::error!(target: "pas::hl7v2::mllp", "accept: {e}");
                    return Err(Error::Streaming(format!("MLLP accept: {e}")));
                }
            };
            let Ok(slot) = slots.clone().try_acquire_owned() else {
                tracing::warn!(target: "pas::hl7v2::mllp", "{peer}: refused, {} connections already open", self.options.max_connections);
                drop(sock);
                continue;
            };
            tracing::debug!(target: "pas::hl7v2::mllp", "new MLLP connection from {peer}");
            let router = self.router.clone();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let _slot = slot;
                let result = match acceptor {
                    None => handle_connection(sock, router, idle).await,
                    Some(acceptor) => match tokio::time::timeout(idle, acceptor.accept(sock)).await
                    {
                        Ok(Ok(tls)) => handle_connection(tls, router, idle).await,
                        Ok(Err(e)) => Err(Error::Streaming(format!("TLS handshake: {e}"))),
                        Err(_) => Err(Error::Streaming("TLS handshake timed out".into())),
                    },
                };
                if let Err(e) = result {
                    tracing::warn!(target: "pas::hl7v2::mllp", "{peer} connection: {e}");
                }
            });
        }
    }
}

/// Read frames from one connection until it closes, goes idle for `idle`, or sends something that is not
/// MLLP; for each frame, dispatch and write the ACK back as a fresh MLLP frame. An oversize frame is answered
/// with an AE and the connection closed, as the stream cannot be read past it.
async fn handle_connection<S>(stream: S, router: Router, idle: Duration) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (rd, mut wr) = tokio::io::split(stream);
    // Buffered: the frame reader takes one byte at a time, which unbuffered is one system call per byte, so a
    // 1 MiB message took long enough on a busy server to run into the idle timeout.
    let mut rd = tokio::io::BufReader::with_capacity(64 * 1024, rd);
    loop {
        let frame = match tokio::time::timeout(idle, mllp::read_frame_checked(&mut rd)).await {
            Err(_) => {
                tracing::debug!(target: "pas::hl7v2::mllp", "idle for {idle:?}; closed");
                return Ok(());
            }
            Ok(result) => result?,
        };
        match frame {
            Frame::Message(f) => {
                let ack = dispatch_frame(&router, &f).await;
                mllp::write_frame(&mut wr, ack.as_bytes()).await?;
            }
            Frame::Closed => return Ok(()),
            Frame::NotMllp => {
                tracing::debug!(target: "pas::hl7v2::mllp", "not MLLP; closed");
                return Ok(());
            }
            Frame::TooLarge => {
                let ack = crate::hl7v2::ack::ack_to(
                    &crate::hl7v2::ack::Inbound::unknown(),
                    crate::hl7v2::AckCode::AppError,
                    Some("message too large"),
                );
                mllp::write_frame(&mut wr, ack.as_bytes()).await?;
                // Close gently: stop writing, then read and drop what the sender is still sending (at most a
                // few MiB, for at most the idle time). Closing with unread input makes TCP reset the
                // connection, which can throw away the AE before the sender reads it.
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let _ = wr.shutdown().await;
                let mut sink = [0u8; 8192];
                let mut left: usize = 8 * 1024 * 1024;
                let _ = tokio::time::timeout(idle, async {
                    while left > 0 {
                        match rd.read(&mut sink).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => left = left.saturating_sub(n),
                        }
                    }
                })
                .await;
                return Ok(());
            }
        }
    }
}

/// Decide which HTTP endpoint to route a frame to, call the router, and
/// return the response body (which is the ACK envelope).
async fn dispatch_frame(router: &Router, frame: &[u8]) -> String {
    let body_str = String::from_utf8_lossy(frame).into_owned();
    // Only the redacted text is ever logged (NFR-33): the header, with every other field masked.
    tracing::debug!(target: "pas::hl7v2::mllp", message = %crate::hl7v2::redact::redact(&body_str), "MLLP frame");
    // Every ACK this listener builds itself answers the message it got (NFR-31), when it can be read.
    let inbound = || {
        parse_message(&body_str)
            .map(|m| crate::hl7v2::ack::Inbound::of(&m))
            .unwrap_or_else(|_| crate::hl7v2::ack::Inbound::unknown())
    };
    let Some(path) = route_for_payload(&body_str) else {
        return crate::hl7v2::ack::ack_to(
            &inbound(),
            crate::hl7v2::AckCode::Reject,
            Some("unsupported message type"),
        );
    };
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/hl7-v2")
        .body(Body::from(body_str.clone()));
    // A failure here is ours, not the sender's: AE, so it is sent again, and no internal text in MSA-3.
    let internal = |what: &str| {
        tracing::error!(target: "pas::hl7v2::mllp", "{what} for {path}");
        crate::hl7v2::ack::ack_to(
            &inbound(),
            crate::hl7v2::AckCode::AppError,
            Some("internal error; send again"),
        )
    };
    let Ok(req) = req else {
        return internal("could not build the request");
    };
    let resp = match router.clone().oneshot(req).await {
        Ok(r) => r,
        Err(e) => return internal(&format!("router oneshot: {e}")),
    };
    let bytes = match axum::body::to_bytes(resp.into_body(), 4 * 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => return internal(&format!("read response: {e}")),
    };
    String::from_utf8(bytes.to_vec()).unwrap_or_else(|_| internal("non-UTF-8 response"))
}

/// Pick the `/api/hl7/v2/<X>` endpoint for the given payload. If the payload
/// fails to parse, the patient endpoint will return an AR, as it would over
/// plain HTTP. A message type the PAS does not handle is `None`: it is
/// refused with an AR, never treated as an add-person (as it was until
/// 2026-11-17).
fn route_for_payload(body: &str) -> Option<&'static str> {
    // FHS/BHS-prefixed payloads carry a batch envelope; route them to the
    // batch endpoint, which dispatches each contained message internally.
    if crate::hl7v2::looks_like_batch(body) {
        return Some("/api/hl7/v2/batch");
    }
    let msg = match parse_message(body) {
        Ok(m) => m,
        Err(_) => return Some("/api/hl7/v2/patient"),
    };
    let (code, event) = message_type(&msg);
    let path = match (code.as_str(), event.as_str()) {
        ("ADT", "A28") => "/api/hl7/v2/patient",
        ("ADT", "A01") => "/api/hl7/v2/admit",
        ("ADT", "A02") => "/api/hl7/v2/transfer",
        ("ADT", "A03") => "/api/hl7/v2/discharge",
        ("ADT", "A04") => "/api/hl7/v2/register",
        ("ADT", "A05") => "/api/hl7/v2/pre-admit",
        ("ADT", "A06") => "/api/hl7/v2/change-to-inpatient",
        ("ADT", "A07") => "/api/hl7/v2/change-to-outpatient",
        ("ADT", "A21") => "/api/hl7/v2/leave-start",
        ("ADT", "A22") => "/api/hl7/v2/leave-end",
        ("ADT", "A23") => "/api/hl7/v2/delete-patient",
        ("ADT", "A38") => "/api/hl7/v2/cancel-pre-admit",
        ("ADT", "A08") => "/api/hl7/v2/update",
        ("ADT", "A11") => "/api/hl7/v2/cancel-admit",
        ("ADT", "A12") => "/api/hl7/v2/cancel-transfer",
        ("ADT", "A13") => "/api/hl7/v2/cancel-discharge",
        ("ADT", "A40") => "/api/hl7/v2/merge",
        ("DFT", "P03") => "/api/hl7/v2/dft",
        ("MFN", "M02") => "/api/hl7/v2/mfn-staff",
        ("MFN", "M05") => "/api/hl7/v2/mfn-location",
        ("SIU", "S12") => "/api/hl7/v2/schedule-book",
        ("SIU", "S13") => "/api/hl7/v2/schedule-reschedule",
        ("SIU", "S14") => "/api/hl7/v2/schedule-modify",
        ("SIU", "S15") => "/api/hl7/v2/schedule-cancel",
        _ => return None,
    };
    Some(path)
}

/// Public entry-point usable in tests: dispatch one MLLP frame end-to-end
/// against any `AsyncRead + AsyncWrite` pair. Useful for in-process round-
/// trip tests that don't want to bind a real socket.
pub async fn handle_one_frame<RW>(stream: &mut RW, router: Router) -> Result<()>
where
    RW: AsyncRead + AsyncWrite + Unpin,
{
    let frame = match mllp::read_frame(stream).await? {
        Some(f) => f,
        None => return Ok(()),
    };
    let ack = dispatch_frame(&router, &frame).await;
    mllp::write_frame(stream, ack.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_route_for_payload_a01() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A01|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/admit"));
    }

    #[test]
    fn test_route_for_payload_a02() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A02|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/transfer"));
    }

    #[test]
    fn test_route_for_payload_a03() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A03|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/discharge"));
    }

    #[test]
    fn test_route_for_payload_a04() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A04|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/register"));
    }

    #[test]
    fn test_route_for_payload_a05() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A05|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/pre-admit"));
    }

    #[test]
    fn test_route_for_payload_a06() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A06|X|P|2.5\r";
        assert_eq!(
            route_for_payload(body),
            Some("/api/hl7/v2/change-to-inpatient")
        );
    }

    #[test]
    fn test_route_for_payload_a07() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A07|X|P|2.5\r";
        assert_eq!(
            route_for_payload(body),
            Some("/api/hl7/v2/change-to-outpatient")
        );
    }

    #[test]
    fn test_route_for_payload_a38() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A38|X|P|2.5\r";
        assert_eq!(
            route_for_payload(body),
            Some("/api/hl7/v2/cancel-pre-admit")
        );
    }

    #[test]
    fn test_route_for_payload_a21() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A21|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/leave-start"));
    }

    #[test]
    fn test_route_for_payload_a22() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A22|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/leave-end"));
    }

    #[test]
    fn test_route_for_payload_a23() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A23|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/delete-patient"));
    }

    #[test]
    fn test_route_for_payload_a28_defaults_to_patient() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A28|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/patient"));
    }

    #[test]
    fn test_route_for_payload_a08() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A08|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/update"));
    }

    #[test]
    fn test_route_for_payload_a11() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A11|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/cancel-admit"));
    }

    #[test]
    fn test_route_for_payload_a12() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A12|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/cancel-transfer"));
    }

    #[test]
    fn test_route_for_payload_a13() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A13|X|P|2.5\r";
        assert_eq!(
            route_for_payload(body),
            Some("/api/hl7/v2/cancel-discharge")
        );
    }

    #[test]
    fn test_route_for_payload_a40() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||ADT^A40|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/merge"));
    }

    #[test]
    fn test_route_for_payload_dft_p03() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||DFT^P03|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/dft"));
    }

    #[test]
    fn test_route_for_payload_mfn_m02() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||MFN^M02|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/mfn-staff"));
    }

    #[test]
    fn test_route_for_payload_mfn_m05() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||MFN^M05|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/mfn-location"));
    }

    #[test]
    fn test_route_for_payload_siu_s12() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||SIU^S12|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/schedule-book"));
    }

    #[test]
    fn test_route_for_payload_siu_s15() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||SIU^S15|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/schedule-cancel"));
    }

    #[test]
    fn test_route_for_payload_siu_s13() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||SIU^S13|X|P|2.5\r";
        assert_eq!(
            route_for_payload(body),
            Some("/api/hl7/v2/schedule-reschedule")
        );
    }

    #[test]
    fn test_route_for_payload_siu_s14() {
        let body = "MSH|^~\\&|S|F|R|F|20260523120000||SIU^S14|X|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/schedule-modify"));
    }

    #[test]
    fn test_route_for_payload_unsupported_types_are_refused_not_added_as_people() {
        // Until 2026-11-17 these fell back to /patient, which has no type check, so a message of any
        // type with a PID made a person. Now the listener answers AR itself.
        for t in ["SIU^S17", "ORU^R01", "ADT^A99"] {
            let body = format!("MSH|^~\\&|S|F|R|F|20260523120000||{t}|X|P|2.5\rPID|1||MRN-1\r");
            assert_eq!(route_for_payload(&body), None, "{t}");
        }
    }

    #[test]
    fn test_route_for_payload_bhs_routes_to_batch() {
        let body = "BHS|^~\\&|S|F|R|F|20260523120000||BATCH-1|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/batch"));
    }

    #[test]
    fn test_route_for_payload_fhs_routes_to_batch() {
        let body = "FHS|^~\\&|S|F|R|F|20260523120000||FILE-1|P|2.5\r";
        assert_eq!(route_for_payload(body), Some("/api/hl7/v2/batch"));
    }

    #[test]
    fn test_route_for_payload_garbage_routes_to_patient() {
        assert_eq!(route_for_payload("not-v2"), Some("/api/hl7/v2/patient"));
    }

    fn fixture_path(name: &str) -> String {
        format!("{}/tests/fixtures/tls/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn the_message_type_is_msh_9_or_empty() {
        let m = parse_message("MSH|^~\\&|S|F|R|F|20261117||ADT^A01|X|P|2.5\rPID|1\r").unwrap();
        assert_eq!(message_type(&m), ("ADT".to_string(), "A01".to_string()));
        let no_event = parse_message("MSH|^~\\&|S|F|R|F|20261117||ACK|X|P|2.5\r").unwrap();
        assert_eq!(message_type(&no_event), ("ACK".to_string(), String::new()));
        assert!(
            parse_message("PID|1\r").is_err(),
            "a message must start with MSH"
        );
    }

    #[test]
    fn the_listener_reports_off_until_it_is_started() {
        // The state is process-wide; only the names are checked, in the order a listener goes through them.
        for (state, name) in [(ACCEPTING, "accepting"), (STOPPED, "stopped"), (OFF, "off")] {
            set_state(state);
            assert_eq!(listener_state(), name);
        }
    }

    #[test]
    fn settings_without_tls_are_plain_and_a_client_ca_alone_is_refused() {
        let o = MllpOptions::from_settings(None, None, None, Duration::from_secs(7), 3).unwrap();
        assert!(o.tls.is_none());
        assert_eq!((o.idle, o.max_connections), (Duration::from_secs(7), 3));
        let d = MllpOptions::default();
        assert_eq!((d.idle, d.max_connections), (Duration::from_secs(30), 64));
        let e = MllpOptions::from_settings(None, None, Some(fixture_path("ca.pem")), d.idle, 1);
        assert!(e.is_err(), "a client CA needs a certificate and key");
    }

    #[test]
    fn a_certificate_without_a_key_or_a_file_that_cannot_be_read_stops_start_up() {
        let (cert, key, ca) = (
            fixture_path("server.pem"),
            fixture_path("server.key"),
            fixture_path("ca.pem"),
        );
        let idle = Duration::from_secs(1);
        assert!(MllpOptions::from_settings(Some(cert.clone()), None, None, idle, 1).is_err());
        assert!(MllpOptions::from_settings(None, Some(key.clone()), None, idle, 1).is_err());
        for (c, k, a) in [
            ("/no/such/cert", key.as_str(), None),
            (cert.as_str(), "/no/such/key", None),
            (cert.as_str(), key.as_str(), Some("/no/such/ca")),
        ] {
            let r = MllpOptions::from_settings(
                Some(c.into()),
                Some(k.into()),
                a.map(String::from),
                idle,
                1,
            );
            assert!(r.is_err(), "{c} {k} {a:?}");
        }
        let ok = MllpOptions::from_settings(Some(cert), Some(key), Some(ca), idle, 2).unwrap();
        assert!(ok.tls.is_some());
    }

    #[test]
    fn a_tls_configuration_refuses_pem_that_is_not_a_certificate_or_a_key() {
        let cert = std::fs::read(fixture_path("server.pem")).unwrap();
        let key = std::fs::read(fixture_path("server.key")).unwrap();
        assert!(
            tls_server_config(b"", &key, None).is_err(),
            "no certificate"
        );
        assert!(tls_server_config(&cert, b"not a key", None).is_err());
        assert!(
            tls_server_config(&cert, &key, Some(b"garbage")).is_err(),
            "a client CA file with no certificate in it trusts nobody"
        );
        let other_key = std::fs::read(fixture_path("client.key")).unwrap();
        assert!(
            tls_server_config(&cert, &other_key, None).is_err(),
            "a key that is not the certificate's"
        );
    }

    #[tokio::test]
    async fn one_frame_is_answered_and_a_closed_stream_is_not_an_error() {
        let router = Router::new();
        let (mut client, mut server) = tokio::io::duplex(8192);
        let msg = "MSH|^~\\&|S|F|R|F|20261117||ZZZ^Z01|MSG-1|P|2.5\r";
        mllp::write_frame(&mut client, msg.as_bytes())
            .await
            .unwrap();
        handle_one_frame(&mut server, router.clone()).await.unwrap();
        let ack = mllp::read_frame(&mut client).await.unwrap().unwrap();
        let ack = String::from_utf8(ack).unwrap();
        assert!(ack.contains("MSA|A"), "an ACK of some kind: {ack}");
        drop(client);
        let (client2, mut server2) = tokio::io::duplex(64);
        drop(client2);
        handle_one_frame(&mut server2, router).await.unwrap();
    }
}
