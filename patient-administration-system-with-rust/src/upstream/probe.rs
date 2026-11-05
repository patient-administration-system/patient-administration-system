//! What `GET /api/health` says about `person-service` and `worker-service`.
//!
//! The health route is open and exempt from the rate limit, so a call to it must not turn into a call to
//! the identity services: the answer is kept for [`PROBE_CACHE`], and callers that arrive during a probe
//! wait for it instead of starting their own. A service that hangs costs at most [`PROBE_TIMEOUT`].

use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use super::{PersonClient, WorkerClient};
use crate::Result;

/// How long a health check waits for one upstream service.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
/// How long one answer is reused.
pub const PROBE_CACHE: Duration = Duration::from_secs(5);

/// What the last probe found for one service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamState {
    /// It answered with a success status.
    Ok,
    /// It did not answer in time, or answered with an error.
    Unreachable,
    /// Its URL is not set, so there was nothing to ask.
    NotConfigured,
}

impl UpstreamState {
    /// The value in the health response.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Unreachable => "unreachable",
            Self::NotConfigured => "not_configured",
        }
    }
}

/// Both services, as `GET /api/health` reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpstreamStatus {
    pub person_service: UpstreamState,
    pub worker_service: UpstreamState,
}

impl UpstreamStatus {
    /// True when a configured service is not answering. A service that is not configured does not count:
    /// a server with no upstream URLs is a normal way to run the bed and billing flows.
    pub fn is_degraded(self) -> bool {
        self.person_service == UpstreamState::Unreachable
            || self.worker_service == UpstreamState::Unreachable
    }
}

/// Keeps the last answer. One per server; see the module notes.
#[derive(Debug, Default)]
pub struct UpstreamProbe {
    last: Mutex<Option<(Instant, UpstreamStatus)>>,
}

impl UpstreamProbe {
    /// The state of both services, from the last answer if it is recent enough.
    pub async fn check(
        &self,
        person: Option<&Arc<dyn PersonClient>>,
        worker: Option<&Arc<dyn WorkerClient>>,
    ) -> UpstreamStatus {
        let mut last = self.last.lock().await;
        if let Some((at, status)) = *last
            && at.elapsed() < PROBE_CACHE
        {
            return status;
        }
        let (person_service, worker_service) = tokio::join!(
            state_of("person-service", person.map(|client| client.ping())),
            state_of("worker-service", worker.map(|client| client.ping())),
        );
        let status = UpstreamStatus {
            person_service,
            worker_service,
        };
        *last = Some((Instant::now(), status));
        status
    }
}

async fn state_of(name: &str, ping: Option<impl Future<Output = Result<()>>>) -> UpstreamState {
    let Some(ping) = ping else {
        return UpstreamState::NotConfigured;
    };
    match tokio::time::timeout(PROBE_TIMEOUT, ping).await {
        Ok(Ok(())) => UpstreamState::Ok,
        Ok(Err(e)) => {
            tracing::warn!(service = name, error = %e, "upstream health check failed");
            UpstreamState::Unreachable
        }
        Err(_) => {
            tracing::warn!(service = name, "upstream health check timed out");
            UpstreamState::Unreachable
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;
    use crate::upstream::{HttpPersonClient, HttpWorkerClient};

    fn person(url: String) -> Arc<dyn PersonClient> {
        Arc::new(HttpPersonClient::new(url, Duration::from_secs(30), None).expect("client"))
    }

    fn worker(url: String) -> Arc<dyn WorkerClient> {
        Arc::new(HttpWorkerClient::new(url, Duration::from_secs(30), None).expect("client"))
    }

    /// A server that answers every request with `status_line` and counts them. `None` never answers.
    async fn server(status_line: Option<&'static str>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut request = [0u8; 2048];
                    let _ = socket.read(&mut request).await;
                    counter.fetch_add(1, Ordering::SeqCst);
                    match status_line {
                        Some(line) => {
                            let response =
                                format!("{line}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                            let _ = socket.write_all(response.as_bytes()).await;
                        }
                        None => tokio::time::sleep(Duration::from_secs(60)).await,
                    }
                });
            }
        });
        (url, hits)
    }

    /// A port nothing listens on.
    async fn closed_port() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("address"));
        drop(listener);
        url
    }

    #[tokio::test]
    async fn a_service_that_answers_is_ok() {
        let (person_url, _) = server(Some("HTTP/1.1 200 OK")).await;
        let (worker_url, _) = server(Some("HTTP/1.1 200 OK")).await;
        let status = UpstreamProbe::default()
            .check(Some(&person(person_url)), Some(&worker(worker_url)))
            .await;
        assert_eq!(status.person_service, UpstreamState::Ok);
        assert_eq!(status.worker_service, UpstreamState::Ok);
        assert!(!status.is_degraded());
    }

    #[tokio::test]
    async fn a_closed_port_is_unreachable_and_degrades() {
        let status = UpstreamProbe::default()
            .check(Some(&person(closed_port().await)), None)
            .await;
        assert_eq!(status.person_service, UpstreamState::Unreachable);
        assert!(status.is_degraded());
    }

    #[tokio::test]
    async fn an_error_status_is_unreachable() {
        let (url, _) = server(Some("HTTP/1.1 500 Internal Server Error")).await;
        let status = UpstreamProbe::default()
            .check(None, Some(&worker(url)))
            .await;
        assert_eq!(status.worker_service, UpstreamState::Unreachable);
        assert!(status.is_degraded());
    }

    #[tokio::test]
    async fn a_service_that_hangs_costs_no_more_than_the_timeout() {
        let (url, hits) = server(None).await;
        let started = Instant::now();
        let status = UpstreamProbe::default()
            .check(Some(&person(url)), None)
            .await;
        assert_eq!(status.person_service, UpstreamState::Unreachable);
        assert!(started.elapsed() < PROBE_TIMEOUT + Duration::from_secs(1));
        assert_eq!(hits.load(Ordering::SeqCst), 1, "the request did reach it");
    }

    #[tokio::test]
    async fn a_service_with_no_url_is_not_a_fault() {
        let status = UpstreamProbe::default().check(None, None).await;
        assert_eq!(status.person_service, UpstreamState::NotConfigured);
        assert_eq!(status.worker_service, UpstreamState::NotConfigured);
        assert!(!status.is_degraded());
    }

    #[tokio::test]
    async fn repeated_checks_reach_the_service_once() {
        let (url, hits) = server(Some("HTTP/1.1 200 OK")).await;
        let client = person(url);
        let probe = Arc::new(UpstreamProbe::default());
        let checks: Vec<_> = (0..20)
            .map(|_| {
                let (probe, client) = (probe.clone(), client.clone());
                tokio::spawn(async move { probe.check(Some(&client), None).await })
            })
            .collect();
        for check in checks {
            check.await.expect("check");
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}
