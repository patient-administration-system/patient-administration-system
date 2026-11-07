//! A request body that may be left out.
//!
//! Handlers read bodies, paths and queries with `axa::extract::{Json, Path, Query}`, which refuse a bad request
//! with a problem quoting nothing the caller sent. Axum lets a handler take `Option<Json<T>>`, but axa's `Json`
//! has no such form, and this crate cannot add one to it, so [`OptionalJson`] is that form.

use axa::extract::Json;
use axum::{
    extract::{FromRequest, Request},
    http::header::CONTENT_TYPE,
};
use serde::de::DeserializeOwned;

/// A JSON body, or `None` when the request has no `content-type`, as Axum's `Option<Json<T>>`. A body that
/// is sent is read by axa's [`Json`], so a malformed one is refused, not taken as missing.
#[derive(Debug, Clone, Default)]
pub struct OptionalJson<T>(pub Option<T>);

impl<T, S> FromRequest<S> for OptionalJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = axa::Error;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        if !req.headers().contains_key(CONTENT_TYPE) {
            return Ok(Self(None));
        }
        let Json(value) = Json::<T>::from_request(req, state).await?;
        Ok(Self(Some(value)))
    }
}

#[cfg(test)]
mod tests {
    use axum::{Router, body::Body, http::StatusCode, routing::post};
    use serde::Deserialize;
    use tower::ServiceExt;

    use super::*;

    #[derive(Deserialize)]
    struct Note {
        text: String,
    }

    async fn call(request: Request) -> (StatusCode, String) {
        let app = Router::new().route(
            "/",
            post(|OptionalJson(note): OptionalJson<Note>| async move {
                note.map_or_else(|| "none".to_string(), |n| n.text)
            }),
        );
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    fn json(body: &str) -> Request {
        Request::post("/")
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn no_body_is_none_and_a_body_is_read() {
        let empty = Request::post("/").body(Body::empty()).unwrap();
        assert_eq!(call(empty).await, (StatusCode::OK, "none".into()));
        assert_eq!(
            call(json(r#"{"text":"hi"}"#)).await,
            (StatusCode::OK, "hi".into())
        );
    }

    #[tokio::test]
    async fn a_malformed_body_is_a_problem_that_quotes_nothing() {
        let (status, body) = call(json("{secret")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.contains("\"code\":\"bad_request\""), "{body}");
        assert!(body.contains("detail"), "{body}");
        assert!(!body.contains("secret"), "{body}");
    }
}
