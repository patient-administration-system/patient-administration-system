//! HUB-5, NFR-33: personal data never reaches a log line or an error response. Needs PostgreSQL:
//! `cargo test -- --ignored`.
//!
//! A marker stands in for personal data. It is sent where a sender puts a patient's identifier, name or other
//! values, down every error path these reach: HL7 v2 messages that cannot be parsed or processed, and REST and
//! FHIR bodies that are refused. The marker must come back in no response body and appear in no log line
//! (everything at TRACE is captured).
mod common;
use axum::{
    body::{Body, to_bytes},
    http::Request,
};
use patient_administration_system::upstream::StubPersonClient;
use std::io::Write;
use std::sync::{Arc, Mutex, OnceLock};
use tower::ServiceExt;

const NEEDLE: &str = "NEEDLE7391";

/// Every log line of this test binary.
fn logs() -> &'static Arc<Mutex<Vec<u8>>> {
    static LOGS: OnceLock<Arc<Mutex<Vec<u8>>>> = OnceLock::new();
    LOGS.get_or_init(|| {
        let buf: Arc<Mutex<Vec<u8>>> = Arc::default();
        let writer = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || Capture(writer.clone()))
            .finish();
        tracing::subscriber::set_global_default(subscriber).unwrap();
        buf
    })
}

struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

async fn call(
    state: &patient_administration_system::api::rest::AppState,
    method: &str,
    path: &str,
    content_type: &str,
    body: String,
) -> String {
    let app = patient_administration_system::api::rest::router(state.clone()).merge(
        patient_administration_system::api::fhir::routes::router(state.clone()),
    );
    let resp = app
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", content_type)
                .header("x-user-id", "hub5-test")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    format!(
        "{status} {}",
        String::from_utf8_lossy(&to_bytes(resp.into_body(), 1 << 20).await.unwrap())
    )
}

fn msh(t: &str) -> String {
    format!(
        "MSH|^~\\&|EMR|WARDS|PAS|HOSP|20261117120000||{t}|HUB5-{}|P|2.5\r",
        uuid::Uuid::new_v4().simple()
    )
}

fn pid() -> String {
    format!("PID|1||{NEEDLE}^^^HOSP^MR||{NEEDLE}^{NEEDLE}||19900115|F|||{NEEDLE} Street^^Town\r")
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn the_marker_reaches_no_response_and_no_log_line() {
    let logs = logs();
    let state = common::build_state(&common::database_url().unwrap())
        .await
        .with_person_client(Arc::new(StubPersonClient::new()));
    let hl7 = "application/hl7-v2";
    let json = "application/json";
    let get_path = format!("/api/appointments/{NEEDLE}");
    let book = format!("/api/slots/{}/book", uuid::Uuid::new_v4());
    let cases: Vec<(&str, &str, &str, String)> = vec![
        // HL7 v2: will not parse; does not start with MSH; segments before MSH in a batch.
        (
            "POST",
            "/api/hl7/v2/admit",
            hl7,
            format!("{NEEDLE} not a message"),
        ),
        (
            "POST",
            "/api/hl7/v2/admit",
            hl7,
            format!("PID|1||{NEEDLE}\r"),
        ),
        (
            "POST",
            "/api/hl7/v2/batch",
            hl7,
            format!("BHS|^~\\&|E|F|P|H|20261117\rPID|{NEEDLE}\rBTS|0\r"),
        ),
        // An admission to a bed that does not exist, named by the marker.
        (
            "POST",
            "/api/hl7/v2/admit",
            hl7,
            format!("{}{}PV1|1|I|W^R^{NEEDLE}\r", msh("ADT^A01"), pid()),
        ),
        // A transfer, a discharge and an update for a patient with no admission.
        (
            "POST",
            "/api/hl7/v2/transfer",
            hl7,
            format!("{}{}PV1|1|I|W^R^{NEEDLE}\r", msh("ADT^A02"), pid()),
        ),
        (
            "POST",
            "/api/hl7/v2/discharge",
            hl7,
            format!("{}{}PV1|1|I\r", msh("ADT^A03"), pid()),
        ),
        // A merge whose source patient is the marker.
        (
            "POST",
            "/api/hl7/v2/merge",
            hl7,
            format!("{}{}MRG|{NEEDLE}-OLD^^^HOSP^MR\r", msh("ADT^A40"), pid()),
        ),
        // A schedule change for an appointment the marker names.
        (
            "POST",
            "/api/hl7/v2/schedule-modify",
            hl7,
            format!("{}SCH|{NEEDLE}|{NEEDLE}\r{}", msh("SIU^S14"), pid()),
        ),
        // A charge whose amount is the marker.
        (
            "POST",
            "/api/hl7/v2/dft",
            hl7,
            format!(
                "{}EVN|P03\r{}PV1|1|O\rFT1|1|||20261117||CG|X1^Charge|Desc||1|{NEEDLE}^GBP\r",
                msh("DFT^P03"),
                pid()
            ),
        ),
        // A type the PAS does not handle.
        (
            "POST",
            "/api/hl7/v2/admit",
            hl7,
            format!("{}{}", msh("ORU^R01"), pid()),
        ),
        // REST: values of the wrong type and shape.
        (
            "POST",
            book.as_str(),
            json,
            format!(r#"{{"person_id":"{NEEDLE}"}}"#),
        ),
        (
            "POST",
            book.as_str(),
            json,
            format!(r#"{{"person_id":{{"{NEEDLE}":1}}}}"#),
        ),
        ("POST", book.as_str(), json, format!("{{{NEEDLE}")),
        ("GET", get_path.as_str(), json, String::new()),
        // FHIR: a reference that is not one, and a body that is not JSON.
        (
            "POST",
            "/fhir/r4/Appointment/$book",
            "application/fhir+json",
            format!(
                r#"{{"resourceType":"Parameters","parameter":[{{"name":"appointment-reference","valueReference":{{"reference":"Appointment/{NEEDLE}"}}}},{{"name":"comment","valueString":"{NEEDLE}"}}]}}"#
            ),
        ),
        (
            "POST",
            "/fhir/r4/Appointment/$book",
            "application/fhir+json",
            NEEDLE.to_string(),
        ),
        (
            "POST",
            "/fhir",
            "application/fhir+json",
            format!(
                r#"{{"resourceType":"Bundle","type":"transaction","entry":[{{"resource":{{"resourceType":"{NEEDLE}"}}}}]}}"#
            ),
        ),
    ];
    let mut leaks = Vec::new();
    for (method, path, ct, body) in cases {
        let path = path.to_string();
        let answer = call(&state, method, &path, ct, body).await;
        if answer.contains(NEEDLE) {
            leaks.push(format!("{method} {path} answered: {answer}"));
        }
    }
    let text = String::from_utf8_lossy(&logs.lock().unwrap()).into_owned();
    for line in text.lines().filter(|l| l.contains(NEEDLE)) {
        leaks.push(format!("log: {line}"));
    }
    assert!(leaks.is_empty(), "the marker leaked:\n{}", leaks.join("\n"));
}

/// PA-02: the log-only senders a development deployment switches on record that a message went, never to whom
/// or what it said.
#[tokio::test]
async fn the_log_only_senders_log_no_recipient_subject_or_text() {
    use patient_administration_system::communication::{
        LogMessageSender, LogSmsProvider, MessageSender, SmsProvider,
    };
    let buf = logs().clone();
    LogMessageSender { channel: "email" }
        .send(
            &format!("{NEEDLE}@example.org"),
            &format!("Subject {NEEDLE}"),
            &format!("Body {NEEDLE}"),
        )
        .await
        .unwrap();
    LogSmsProvider
        .send(&format!("+4477009{NEEDLE}"), &format!("Text {NEEDLE}"))
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&buf.lock().unwrap()).to_string();
    assert!(
        text.contains("message dispatched (log-only)"),
        "the sender still says it ran: {text}"
    );
    assert!(text.contains("SMS dispatched (LogSmsProvider)"), "{text}");
    assert!(!text.contains(NEEDLE), "personal data in the log: {text}");
}
