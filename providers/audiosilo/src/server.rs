use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde_json::json;

use crate::audiosilo::{AudioSilo, Query as SearchQuery, UpstreamError};
use crate::VERSION;

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 10;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(9);

#[derive(Clone)]
struct AppState {
    source: Arc<AudioSilo>,
    auth_token: Option<String>,
}

pub fn routes(source: Arc<AudioSilo>, auth_token: Option<String>) -> Router {
    Router::new()
        .route("/provider/info", get(info))
        .route("/search", get(search))
        .route("/book/{id}", get(lookup))
        .with_state(AppState { source, auth_token })
}

async fn info(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "name": "AudioSilo",
        "version": VERSION,
        "avatarUrl": "https://meta.audiosilo.app/apple-touch-icon.png",
        "color": "#d46a45",
        "authenticationRequired": state.auth_token.is_some(),
        "key": "audiosilo",
        "mediaTypes": ["audiobook"],
        "lookup": true,
        "supportedFields": [
            "title", "subtitle", "description", "cover", "authors", "narrators", "publisher",
            "publishedYear", "language", "duration", "genres", "seriesName", "seriesIndex", "isbn"
        ],
        "searchParams": ["isbn", "limit"],
        "bookUrlTemplate": "https://meta.audiosilo.app/works/{id}",
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
    let query = SearchQuery {
        title: param("query").to_string(),
        author: param("author").to_string(),
        isbn: param("isbn").to_string(),
        media_type: param("mediaType").to_string(),
        limit: param("limit")
            .parse::<usize>()
            .map_or(DEFAULT_LIMIT, |limit| limit.clamp(1, MAX_LIMIT)),
    };
    if query.title.is_empty() && query.isbn.is_empty() {
        return error(StatusCode::BAD_REQUEST, "query is required");
    }
    match tokio::time::timeout(REQUEST_TIMEOUT, state.source.search(&query)).await {
        Ok(Ok(answer)) => Json(answer).into_response(),
        Ok(Err(err)) => upstream_error("search", err),
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
    match tokio::time::timeout(REQUEST_TIMEOUT, state.source.lookup(&id)).await {
        Ok(Ok(Some(book))) => Json(book).into_response(),
        Ok(Ok(None)) => error(StatusCode::NOT_FOUND, "Not found"),
        Ok(Err(err)) => upstream_error("lookup", err),
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
                "AudioSilo is temporarily unavailable",
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after_seconds));
            response
        }
        UpstreamError::Failed(message) => {
            eprintln!("[provider.{operation}] [fail] error={message:?} - AudioSilo request failed");
            error(StatusCode::BAD_GATEWAY, "AudioSilo is unavailable")
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
    fn auth_token_is_optional_and_compared_exactly() {
        let mut headers = HeaderMap::new();
        assert!(deny_unauthorized(None, &headers).is_none());
        assert_eq!(
            deny_unauthorized(Some("secret"), &headers)
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );

        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("secret"));
        assert!(deny_unauthorized(Some("secret"), &headers).is_none());
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Secret"));
        assert_eq!(
            deny_unauthorized(Some("secret"), &headers)
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
}
