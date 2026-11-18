//! hub_bench — Criterion benchmarks for the integration-hub path (HUB-1 to HUB-6), no database (PERF-3).
//!
//! What every inbound HL7 v2 message costs besides parsing (its ACK, its de-duplication key, its redacted log
//! line, the MLLP frame it arrives in), the round trip through the MLLP listener on loopback, plain and over
//! TLS, and what every outbound delivery costs besides the network (the signature, the failure class, the
//! backoff). The listener answers from a stub router, so the round trip is the listener's own cost: framing,
//! the connection limit, TLS, and the hop into the router.

#![cfg_attr(not(test), forbid(unsafe_code))]
use axum::{Router, routing::post};
use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use patient_administration_system::hl7v2::MllpServer;
use patient_administration_system::hl7v2::ack::{AckCode, Inbound, ack_code_and_text, ack_to};
use patient_administration_system::hl7v2::batch::{BatchInbound, encode_batch_ack, parse_batch};
use patient_administration_system::hl7v2::dedupe::Key;
use patient_administration_system::hl7v2::listener::{MllpOptions, tls_server_config};
use patient_administration_system::hl7v2::mllp::{Frame, read_frame, read_frame_checked};
use patient_administration_system::hl7v2::parser::parse_message;
use patient_administration_system::hl7v2::redact::redact;
use patient_administration_system::streaming::dispatcher::backoff;
use patient_administration_system::streaming::webhook::{WebhookEventPublisher, classify};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, ServerName};
use std::hint::black_box;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

const A01: &str = include_str!("../examples/hl7-adt-a01.txt");
const BATCH: &str = include_str!("../examples/hl7-batch-adt.txt");

/// A01 with `n` note segments, as `hl7v2_bench` builds it, so the two files' sizes compare.
fn long_message(n: usize) -> String {
    let mut s = A01.trim_end().replace('\n', "\r");
    for i in 0..n {
        s.push_str(&format!(
            "\rNTE|{i}||A free-text note with components^and^repeats~and escapes \\F\\ \\S\\ number {i}"
        ));
    }
    s.push('\r');
    s
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
}

fn bench_ack(c: &mut Criterion) {
    let msg = parse_message(A01).unwrap();
    let inbound = Inbound::of(&msg);
    let ack = ack_to(&inbound, AckCode::Accept, Some("kept"));
    let mut group = c.benchmark_group("hub_ack");
    group.bench_function("inbound_of_a01", |b| {
        b.iter(|| Inbound::of(black_box(&msg)));
    });
    group.bench_function("ack_to_accept", |b| {
        b.iter(|| ack_to(black_box(&inbound), AckCode::Accept, Some("kept")));
    });
    group.bench_function("ack_to_error_long_text", |b| {
        let text = "x".repeat(500);
        b.iter(|| {
            ack_to(
                black_box(&inbound),
                AckCode::AppError,
                Some(black_box(&text)),
            )
        });
    });
    group.bench_function("ack_code_and_text", |b| {
        b.iter(|| ack_code_and_text(black_box(&ack)).unwrap());
    });
    group.bench_function("dedupe_key", |b| {
        b.iter(|| Key::of(black_box(&inbound)).unwrap());
    });
    group.finish();

    let batch = parse_batch(BATCH).unwrap();
    let batch_inbound = BatchInbound::of(batch.bhs.as_ref());
    let mut group = c.benchmark_group("hub_batch_ack");
    for n in [1, 10, 100, 1000] {
        let acks: Vec<String> = (0..n).map(|_| ack.clone()).collect();
        group.throughput(Throughput::Elements(n as u64));
        group.bench_with_input(BenchmarkId::from_parameter(n), &acks, |b, acks| {
            b.iter(|| encode_batch_ack(black_box(&batch_inbound), black_box(acks)));
        });
    }
    group.finish();
}

fn bench_redact(c: &mut Criterion) {
    let mut group = c.benchmark_group("hub_redact");
    for n in [0, 10, 100, 1000] {
        let msg = long_message(n);
        group.throughput(Throughput::Bytes(msg.len() as u64));
        group.bench_with_input(BenchmarkId::new("segments", n), &msg, |b, m| {
            b.iter(|| redact(black_box(m)));
        });
    }
    group.finish();
}

/// `payload` in an MLLP frame.
fn framed(payload: &[u8]) -> Vec<u8> {
    let mut f = Vec::with_capacity(payload.len() + 3);
    f.push(0x0b);
    f.extend_from_slice(payload);
    f.extend_from_slice(&[0x1c, 0x0d]);
    f
}

fn bench_frame(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("hub_mllp_frame");
    for n in [0, 100, 1000] {
        let frame = framed(long_message(n).as_bytes());
        group.throughput(Throughput::Bytes(frame.len() as u64));
        group.bench_with_input(
            BenchmarkId::new("read_checked", frame.len()),
            &frame,
            |b, f| {
                b.to_async(&rt).iter(move || async move {
                    let mut r = BufReader::new(black_box(f.as_slice()));
                    match read_frame_checked(&mut r).await.unwrap() {
                        Frame::Message(m) => m,
                        _ => unreachable!("a whole frame"),
                    }
                });
            },
        );
    }
    group.finish();
}

const STUB_ACK: &str =
    "MSH|^~\\&|PAS|HOSP|EMR|WARDS|20261117120000||ACK^A01^ACK|X|P|2.5\rMSA|AA|MSG-1\r";

fn stub_router() -> Router {
    Router::new().route("/api/hl7/v2/admit", post(|| async { STUB_ACK }))
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/tls/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .expect("the test TLS fixtures")
}

fn options(tls: bool) -> MllpOptions {
    MllpOptions {
        tls: tls.then(|| {
            Arc::new(
                tls_server_config(&fixture("server.pem"), &fixture("server.key"), None).unwrap(),
            )
        }),
        idle: Duration::from_secs(30),
        max_connections: 64,
    }
}

async fn start(tls: bool) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = MllpServer::new(stub_router()).with_options(options(tls));
    tokio::spawn(async move { server.serve_listener(listener).await });
    addr
}

fn connector() -> tokio_rustls::TlsConnector {
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(&fixture("ca.pem")).unwrap())
        .unwrap();
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
    tokio_rustls::TlsConnector::from(Arc::new(config))
}

/// A client connection that closes with no TIME_WAIT, so a benchmark that opens thousands of connections does
/// not run out of local ports.
async fn connect(addr: SocketAddr) -> TcpStream {
    let socket = tokio::net::TcpSocket::new_v4().unwrap();
    socket.set_zero_linger().unwrap();
    socket.connect(addr).await.unwrap()
}

/// Send one framed message and read the framed reply.
async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(s: &mut BufReader<S>, frame: &[u8]) {
    s.get_mut().write_all(frame).await.unwrap();
    s.get_mut().flush().await.unwrap();
    let reply = read_frame(s).await.unwrap().expect("a reply");
    debug_assert!(reply.starts_with(b"MSH"));
}

fn bench_listener(c: &mut Criterion) {
    let rt = runtime();
    let plain = rt.block_on(start(false));
    let tls = rt.block_on(start(true));
    let frame = framed(A01.replace('\n', "\r").as_bytes());
    let name = ServerName::try_from("localhost").unwrap();
    let mut group = c.benchmark_group("hub_mllp_round_trip");
    group.throughput(Throughput::Elements(1));

    // One connection kept open, one message after another: a sender's steady state.
    group.bench_function("plain_kept_open", |b| {
        let mut s = rt.block_on(async { BufReader::new(TcpStream::connect(plain).await.unwrap()) });
        b.iter_custom(|iters| {
            rt.block_on(async {
                let started = Instant::now();
                for _ in 0..iters {
                    exchange(&mut s, &frame).await;
                }
                started.elapsed()
            })
        });
    });
    group.bench_function("tls_kept_open", |b| {
        let mut s = rt.block_on(async {
            let tcp = TcpStream::connect(tls).await.unwrap();
            BufReader::new(connector().connect(name.clone(), tcp).await.unwrap())
        });
        b.iter_custom(|iters| {
            rt.block_on(async {
                let started = Instant::now();
                for _ in 0..iters {
                    exchange(&mut s, &frame).await;
                }
                started.elapsed()
            })
        });
    });

    // A new connection for each message: what a sender that reconnects every time costs, and the handshake.
    group.sample_size(30);
    group.bench_function("plain_new_connection", |b| {
        b.to_async(&rt).iter(|| async {
            let mut s = BufReader::new(connect(plain).await);
            exchange(&mut s, &frame).await;
        });
    });
    let tls_connector = connector();
    group.bench_function("tls_new_connection", |b| {
        b.to_async(&rt).iter(|| async {
            let tcp = connect(tls).await;
            let mut s = BufReader::new(tls_connector.connect(name.clone(), tcp).await.unwrap());
            exchange(&mut s, &frame).await;
        });
    });
    group.finish();
}

fn bench_outbound(c: &mut Criterion) {
    let mut group = c.benchmark_group("hub_outbound");
    let secret = vec![7u8; 32];
    for size in [256, 4096, 65536] {
        let body = vec![b'x'; size];
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("sign", size), &body, |b, body| {
            b.iter(|| WebhookEventPublisher::sign(black_box(&secret), black_box(body)));
        });
    }
    group.throughput(Throughput::Elements(1));
    let url = "https://receiver.example/hook";
    group.bench_function("classify_accepted", |b| {
        b.iter(|| classify(black_box(url), 204, None));
    });
    group.bench_function("classify_rejected", |b| {
        b.iter(|| classify(black_box(url), 422, None));
    });
    group.bench_function("classify_deferred_retry_after", |b| {
        b.iter(|| classify(black_box(url), 503, Some(black_box("120"))));
    });
    group.bench_function("backoff", |b| {
        b.iter(|| backoff(black_box(7), black_box(None)));
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_ack,
    bench_redact,
    bench_frame,
    bench_listener,
    bench_outbound
);
criterion_main!(benches);
