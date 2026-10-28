use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use crate::graphicaudio::{GraphicAudio, UpstreamError, DEFAULT_LIMIT, MAX_LIMIT};
use crate::VERSION;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_ID_LENGTH: usize = 512;

#[derive(Clone)]
struct AppState {
    source: Arc<GraphicAudio>,
    auth_token: Option<String>,
}

pub fn routes(source: Arc<GraphicAudio>, auth_token: Option<String>) -> Router {
    Router::new()
        .route("/provider/info", get(info))
        .route("/search", get(search))
        .route("/book/{id}", get(lookup))
        .with_state(AppState { source, auth_token })
}

async fn info(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "name": "Graphic Audio",
        "version": VERSION,
        "avatarUrl": "https://www.graphicaudiointernational.net/media/favicon/stores/3/favicon_1.ico",
        "color": "#c8102e",
        "authenticationRequired": state.auth_token.is_some(),
        "key": "graphicaudio",
        "mediaTypes": ["audiobook"],
        "lookup": true,
        "supportedFields": [
            "title", "subtitle", "description", "cover", "authors", "narrators", "publishedYear",
            "genres", "seriesName", "seriesIndex", "isbn"
        ],
        "searchParams": ["isbn", "limit"],
        "rateLimit": { "maxConcurrency": 1, "minIntervalMs": 0 }
    }))
}

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
) -> Response {
    if let Some(denied) = deny_unauthorized(state.auth_token.as_deref(), &headers) {
        return denied;
    }
    let param = |name: &str| {
        params
            .get(name)
            .map(|value| value.trim())
            .unwrap_or_default()
    };
    let title = param("query");
    let isbn = param("isbn");
    if title.is_empty() && isbn.is_empty() {
        return error(StatusCode::BAD_REQUEST, "query or isbn is required");
    }
    if !param("mediaType").is_empty() && param("mediaType") != "audiobook" {
        return Json(json!({ "matches": [] })).into_response();
    }
    let limit = param("limit")
        .parse::<usize>()
        .map_or(DEFAULT_LIMIT, |limit| limit.clamp(1, MAX_LIMIT));
    match tokio::time::timeout(
        REQUEST_TIMEOUT,
        state.source.search(title, param("author"), isbn, limit),
    )
    .await
    {
        Ok(Ok(answer)) => Json(answer).into_response(),
        Ok(Err(error)) => upstream_error("search", error),
        Err(_) => upstream_error("search", UpstreamError::Failed("request timed out".into())),
    }
}

async fn lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = deny_unauthorized(state.auth_token.as_deref(), &headers) {
        return denied;
    }
    if id.len() > MAX_ID_LENGTH {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    match tokio::time::timeout(REQUEST_TIMEOUT, state.source.lookup(&id)).await {
        Ok(Ok(Some(book))) => Json(book).into_response(),
        Ok(Ok(None)) | Ok(Err(UpstreamError::NotFound)) => {
            error(StatusCode::NOT_FOUND, "Not found")
        }
        Ok(Err(error)) => upstream_error("lookup", error),
        Err(_) => upstream_error("lookup", UpstreamError::Failed("request timed out".into())),
    }
}

fn deny_unauthorized(token: Option<&str>, headers: &HeaderMap) -> Option<Response> {
    let token = token?;
    let given = headers
        .get(header::AUTHORIZATION)
        .map(HeaderValue::as_bytes)
        .unwrap_or_default();
    let matches = given.len() == token.len()
        && given
            .iter()
            .zip(token.as_bytes())
            .fold(0u8, |diff, (left, right)| diff | (left ^ right))
            == 0;
    (!matches).then(|| error(StatusCode::UNAUTHORIZED, "Unauthorized"))
}

fn upstream_error(operation: &str, upstream: UpstreamError) -> Response {
    match upstream {
        UpstreamError::NotFound => error(StatusCode::NOT_FOUND, "Not found"),
        UpstreamError::RateLimited {
            retry_after_seconds,
        } => {
            let mut response = error(
                StatusCode::TOO_MANY_REQUESTS,
                "GraphicAudio catalog is rate limiting this provider",
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after_seconds));
            response
        }
        UpstreamError::Failed(message) => {
            eprintln!(
                "[provider.{operation}] [fail] error={message:?} - GraphicAudio request failed"
            );
            error(
                StatusCode::BAD_GATEWAY,
                "GraphicAudio catalog is unavailable",
            )
        }
    }
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_is_optional_and_exact() {
        let headers = HeaderMap::new();
        assert!(deny_unauthorized(None, &headers).is_none());
        assert_eq!(
            deny_unauthorized(Some("secret"), &headers)
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("secret"));
        assert!(deny_unauthorized(Some("secret"), &headers).is_none());
    }
}
