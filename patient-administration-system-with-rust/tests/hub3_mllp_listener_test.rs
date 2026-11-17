//! HUB-3, NFR-31: the MLLP listener closes idle connections, caps how many are open, closes a peer that does
//! not speak MLLP, answers an oversize frame with an AE, refuses a type the PAS does not handle, and offers TLS
//! with optional client certificates. No database: the router is a stub that answers AA.
//!
//! The certificates are test-only (`tests/fixtures/tls/README.md`).
use axum::{Router, routing::post};
use patient_administration_system::hl7v2::listener::{MllpOptions, tls_server_config};
use patient_administration_system::hl7v2::{MllpServer, SegmentRead, parse_message};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const A01: &str = "MSH|^~\\&|EMR|WARDS|PAS|HOSP|20261117120000||ADT^A01|MSG-1|P|2.5\rPID|1||MRN-1\rPV1|1|I|W^R^B1\r";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/tls/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

fn stub_router() -> Router {
    Router::new().route(
        "/api/hl7/v2/admit",
        post(|| async {
            "MSH|^~\\&|PAS|HOSP|EMR|WARDS|20261117120000||ACK^A01^ACK|X|P|2.5\rMSA|AA|MSG-1\r"
        }),
    )
}

async fn start(options: MllpOptions) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = MllpServer::new(stub_router()).with_options(options);
    tokio::spawn(async move { server.serve_listener(listener).await });
    addr
}

fn plain(idle_ms: u64, max: usize) -> MllpOptions {
    MllpOptions {
        tls: None,
        idle: Duration::from_millis(idle_ms),
        max_connections: max,
    }
}

async fn send<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S, payload: &[u8]) {
    s.write_all(&[0x0b]).await.unwrap();
    s.write_all(payload).await.unwrap();
    s.write_all(&[0x1c, 0x0d]).await.unwrap();
    s.flush().await.unwrap();
}

/// The next frame's payload, or `None` when the server closed the connection.
async fn recv<S: AsyncRead + Unpin>(s: &mut S) -> Option<String> {
    let mut out = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), s.read(&mut byte)).await {
            Ok(Ok(0)) | Ok(Err(_)) => return None,
            Ok(Ok(_)) => {}
            Err(_) => panic!("no answer and no close within 5 s"),
        }
        match byte[0] {
            0x0b => out.clear(),
            0x1c => {
                let _ = s.read(&mut byte).await;
                return Some(String::from_utf8(out).unwrap());
            }
            b => out.push(b),
        }
    }
}

fn msa(ack: &str) -> (String, String) {
    let m = parse_message(ack).unwrap();
    let s = m.segment("MSA").unwrap();
    (s.raw(1), s.raw(3))
}

#[tokio::test]
async fn a_message_is_answered_and_an_idle_connection_is_closed() {
    let addr = start(plain(300, 8)).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    send(&mut s, A01.as_bytes()).await;
    assert_eq!(msa(&recv(&mut s).await.unwrap()).0, "AA");
    // Nothing more for longer than the idle time: the server closes.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(recv(&mut s).await, None);
}

#[tokio::test]
async fn a_connection_over_the_cap_is_closed_at_once() {
    let addr = start(plain(5_000, 1)).await;
    let mut first = TcpStream::connect(addr).await.unwrap();
    send(&mut first, A01.as_bytes()).await;
    assert!(recv(&mut first).await.is_some(), "the first is served");
    let mut second = TcpStream::connect(addr).await.unwrap();
    send(&mut second, A01.as_bytes()).await;
    assert_eq!(
        recv(&mut second).await,
        None,
        "the second is refused while the first is open"
    );
    drop(first);
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut third = TcpStream::connect(addr).await.unwrap();
    send(&mut third, A01.as_bytes()).await;
    assert!(recv(&mut third).await.is_some(), "a slot is free again");
}

#[tokio::test]
async fn a_peer_that_does_not_speak_mllp_is_closed_without_an_answer() {
    let addr = start(plain(5_000, 8)).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: x\r\n\r\n")
        .await
        .unwrap();
    assert_eq!(recv(&mut s).await, None);
}

#[tokio::test]
async fn an_oversize_frame_gets_an_ae_and_the_connection_closes() {
    let addr = start(plain(5_000, 8)).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    s.write_all(&[0x0b]).await.unwrap();
    // The server stops reading after the limit, so a write past it may fail: that is fine.
    let _ = s.write_all(&vec![b'x'; 1024 * 1024 + 16]).await;
    let ack = recv(&mut s).await.expect("an AE before the close");
    assert_eq!(msa(&ack), ("AE".into(), "message too large".into()));
    assert_eq!(recv(&mut s).await, None);
}

#[tokio::test]
async fn an_unsupported_type_is_refused_with_an_ar() {
    let addr = start(plain(5_000, 8)).await;
    let mut s = TcpStream::connect(addr).await.unwrap();
    send(
        &mut s,
        b"MSH|^~\\&|LAB|F|PAS|H|20261117120000||ORU^R01|R-1|P|2.5\rPID|1||MRN-1\r",
    )
    .await;
    let ack = recv(&mut s).await.unwrap();
    assert_eq!(msa(&ack), ("AR".into(), "unsupported message type".into()));
}

fn tls_options(client_ca: bool) -> MllpOptions {
    let ca = fixture("ca.pem");
    MllpOptions {
        tls: Some(Arc::new(
            tls_server_config(
                &fixture("server.pem"),
                &fixture("server.key"),
                client_ca.then_some(ca.as_slice()),
            )
            .unwrap(),
        )),
        idle: Duration::from_secs(5),
        max_connections: 8,
    }
}

/// A TLS client that trusts the test CA and presents `client` (`"client"`, `"stranger"`) if given.
fn connector(client: Option<&str>) -> tokio_rustls::TlsConnector {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(&fixture("ca.pem")).unwrap())
        .unwrap();
    let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots);
    let config = match client {
        None => builder.with_no_client_auth(),
        Some(name) => builder
            .with_client_auth_cert(
                vec![CertificateDer::from_pem_slice(&fixture(&format!("{name}.pem"))).unwrap()],
                PrivateKeyDer::from_pem_slice(&fixture(&format!("{name}.key"))).unwrap(),
            )
            .unwrap(),
    };
    tokio_rustls::TlsConnector::from(Arc::new(config))
}

/// Connect over TLS and send A01; the MSA-1 that came back, or `None` when the handshake or the reply failed.
async fn tls_round_trip(addr: SocketAddr, client: Option<&str>) -> Option<String> {
    let tcp = TcpStream::connect(addr).await.unwrap();
    let mut tls = connector(client)
        .connect(ServerName::try_from("localhost").unwrap(), tcp)
        .await
        .ok()?;
    send(&mut tls, A01.as_bytes()).await;
    recv(&mut tls).await.map(|a| msa(&a).0)
}

#[tokio::test]
async fn tls_carries_the_message_and_plain_text_gets_no_answer() {
    let addr = start(tls_options(false)).await;
    assert_eq!(tls_round_trip(addr, None).await.as_deref(), Some("AA"));
    let mut plain = TcpStream::connect(addr).await.unwrap();
    send(&mut plain, A01.as_bytes()).await;
    assert_eq!(
        recv(&mut plain).await,
        None,
        "no ACK in plain text on a TLS port"
    );
}

#[tokio::test]
async fn with_a_client_ca_only_a_sender_it_signed_is_served() {
    let addr = start(tls_options(true)).await;
    assert_eq!(
        tls_round_trip(addr, Some("client")).await.as_deref(),
        Some("AA")
    );
    assert_eq!(tls_round_trip(addr, None).await, None, "no certificate");
    assert_eq!(
        tls_round_trip(addr, Some("stranger")).await,
        None,
        "another CA's certificate"
    );
}

#[test]
fn a_certificate_without_a_key_refuses_to_start() {
    let dir = format!("{}/tests/fixtures/tls", env!("CARGO_MANIFEST_DIR"));
    let idle = Duration::from_secs(30);
    let cert = Some(format!("{dir}/server.pem"));
    let key = Some(format!("{dir}/server.key"));
    let ca = Some(format!("{dir}/ca.pem"));
    let err = |r: patient_administration_system::Result<MllpOptions>| {
        r.err().map(|e| e.to_string()).unwrap_or_default()
    };
    assert!(
        err(MllpOptions::from_settings(
            cert.clone(),
            None,
            None,
            idle,
            8
        ))
        .contains("set together")
    );
    assert!(
        err(MllpOptions::from_settings(None, key.clone(), None, idle, 8)).contains("set together")
    );
    assert!(err(MllpOptions::from_settings(None, None, ca.clone(), idle, 8)).contains("needs"));
    assert!(
        MllpOptions::from_settings(None, None, None, idle, 8)
            .unwrap()
            .tls
            .is_none()
    );
    assert!(
        MllpOptions::from_settings(cert.clone(), key.clone(), ca, idle, 8)
            .unwrap()
            .tls
            .is_some()
    );
    // A key file that is not a key is refused too, not ignored.
    assert!(
        !err(MllpOptions::from_settings(
            cert.clone(),
            cert,
            None,
            idle,
            8
        ))
        .is_empty()
    );
}
