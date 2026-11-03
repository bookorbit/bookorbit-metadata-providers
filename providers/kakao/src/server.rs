use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::json;

use crate::author_normalizer::{author_names_match, normalize_author_query};
use crate::kakao::{Client, UpstreamError};
use crate::mapping::{BookMetadata, KakaoDocument};
use crate::{Config, VERSION};

const DEFAULT_LIMIT: usize = 5;
const MAX_LIMIT: usize = 20;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(25);
const MAX_ID_LENGTH: usize = 64;

#[derive(Clone)]
struct AppState {
    cfg: Arc<Config>,
    kakao: Arc<Client>,
    /// Kakao has no documented lookup-by-ID endpoint; records are available here after search.
    cache: Arc<RwLock<HashMap<String, BookMetadata>>>,
}

pub fn routes(cfg: Arc<Config>, kakao: Arc<Client>) -> Router {
    Router::new()
        .route("/provider/info", get(info))
        .route("/search", get(search))
        .route("/book/{id}", get(lookup))
        .with_state(AppState {
            cfg,
            kakao,
            cache: Arc::new(RwLock::new(HashMap::new())),
        })
}

async fn info(State(state): State<AppState>) -> Response {
    Json(json!({
        "name": "Kakao",
        "version": VERSION,
        "avatarUrl": "https://developers.kakao.com/favicon.ico",
        "specVersion": "1",
        "authenticationRequired": state.cfg.auth_token.is_some(),
        "key": "kakao",
        "mediaTypes": ["book"],
        "lookup": true,
        "supportedFields": ["title", "description", "cover", "authors", "publisher", "publishedYear", "language"],
        "searchParams": ["limit", "author", "target", "sort", "page", "isbn"],
        "idMaxLength": MAX_ID_LENGTH,
        "coverShape": "portrait",
        "color": "#FEE500",
        "rateLimit": { "maxConcurrency": state.cfg.max_inflight_requests, "minIntervalMs": if state.cfg.requests_per_second == 0 { 0 } else { (1000.0 / state.cfg.requests_per_second as f64).ceil() as u64 } }
    })).into_response()
}

#[derive(Serialize)]
struct SearchAnswer {
    matches: Vec<BookMetadata>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    partial: bool,
}

async fn search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HashMap<String, String>>,
) -> Response {
    if let Some(denied) = deny_unauthorized(&state, &headers) {
        return denied;
    }
    let param = |key: &str| q.get(key).map(|v| v.trim()).unwrap_or_default();
    let mut query = param("query").to_string();
    let isbn = param("isbn");
    let mut target = param("target").to_string();
    if query.is_empty() && !isbn.is_empty() {
        query = isbn.to_string();
        target = "isbn".to_string();
    }

    // 인명 검색인 경우 API에 전달하기 전에 역할 표현 제거, 첫번째 이름만 남김
    if target == "person" {
        query = normalize_author_query(&query);
    }

    if query.is_empty() {
        return error(StatusCode::BAD_REQUEST, "query is required");
    }
    if !target.is_empty() && !["title", "isbn", "publisher", "person"].contains(&target.as_str()) {
        return error(StatusCode::BAD_REQUEST, "invalid target");
    }
    let sort = match param("sort") {
        "" => None,
        "accuracy" => Some("accuracy"),
        "latest" => Some("latest"),
        _ => return error(StatusCode::BAD_REQUEST, "invalid sort"),
    };
    let page = q
        .get("page")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 50);
    let limit = q
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .map_or(DEFAULT_LIMIT, |n| n.clamp(1, MAX_LIMIT));
    let size = limit.clamp(1, 50) as u32;
    let target = (!target.is_empty()).then_some(target.as_str());

    let result = tokio::time::timeout(
        REQUEST_TIMEOUT,
        state.kakao.search(&query, target, sort, page, size),
    )
    .await
    .unwrap_or_else(|_| Err(UpstreamError::Failed("request timed out".into())));
    match result {
        Ok(response) => {
            // Keep Kakao pagination metadata intentionally observable to the handler.
            let _pagination = (
                response.meta.total_count,
                response.meta.pageable_count,
                response.meta.is_end,
            );
            let author_filter = param("author").to_lowercase();
            let mut matches: Vec<BookMetadata> = response
                .documents
                .into_iter()
                .filter(|doc| {
                    author_filter.is_empty()
                        || doc
                            .authors
                            .iter()
                            .any(|author| author_names_match(author, &author_filter))
                })
                .map(KakaoDocument::into_metadata)
                .collect();
            matches.truncate(limit);
            if let Ok(mut cache) = state.cache.write() {
                for book in &matches {
                    cache.insert(book.provider_id.clone(), book.clone());
                }
            }
            Json(SearchAnswer {
                matches,
                partial: false,
            })
            .into_response()
        }
        Err(err) => upstream_error("search", err),
    }
}

async fn lookup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
) -> Response {
    if let Some(denied) = deny_unauthorized(&state, &headers) {
        return denied;
    }
    // New provider IDs contain only the SHA-256 hex digest. Accept the legacy
    // KAKAO-prefixed form for old links, but normalize it before cache lookup.
    let normalized_id = id.strip_prefix("KAKAO-").unwrap_or(&id);
    if normalized_id.len() != MAX_ID_LENGTH || !normalized_id.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    match state.cache.read() {
        Ok(cache) => match cache.get(normalized_id) {
            Some(book) => Json(book).into_response(),
            None => error(
                StatusCode::NOT_FOUND,
                "Book not found in provider cache; search for it first",
            ),
        },
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Provider cache unavailable",
        ),
    }
}

fn upstream_error(op: &str, err: UpstreamError) -> Response {
    match err {
        UpstreamError::RateLimited {
            retry_after_seconds,
        } => {
            let mut res = error(StatusCode::TOO_MANY_REQUESTS, "rate limited by Kakao");
            res.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(retry_after_seconds));
            res
        }
        UpstreamError::Unauthorized => {
            error(StatusCode::BAD_GATEWAY, "Kakao API authentication failed")
        }
        UpstreamError::Failed(message) => {
            eprintln!("[provider.{op}] [fail] upstream request failed: {message:?}");
            error(StatusCode::BAD_GATEWAY, "Kakao API unavailable")
        }
    }
}

fn deny_unauthorized(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    let token = state.cfg.auth_token.as_ref()?;
    let given = headers
        .get(header::AUTHORIZATION)
        .map(HeaderValue::as_bytes)
        .unwrap_or_default();
    (!constant_time_eq(given, token.as_bytes()))
        .then(|| error(StatusCode::UNAUTHORIZED, "Unauthorized"))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |diff, (x, y)| diff | (x ^ y)) == 0
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::Query;
    use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};

    const ISBN_SUCCESS: &str = include_str!("../testdata/response-isbn-success.json");
    const TITLE_SUCCESS: &str = include_str!("../testdata/response-title-success.json");

    async fn mock_kakao(status: Arc<AtomicU16>, calls: Arc<AtomicUsize>) -> String {
        let app = Router::new().route(
            "/",
            get(move |Query(_q): Query<HashMap<String, String>>| {
                let status = status.clone();
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    let code = status.load(Ordering::SeqCst);
                    if code != 0 {
                        return (
                            StatusCode::from_u16(code).unwrap(),
                            Json(json!({"error":"mock failure"})),
                        )
                            .into_response();
                    }
                    Json(serde_json::from_str::<serde_json::Value>(TITLE_SUCCESS).unwrap())
                        .into_response()
                }
            }),
        );
        serve(app).await
    }

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{addr}")
    }

    async fn get_json(url: &str) -> (StatusCode, serde_json::Value) {
        let response = reqwest::Client::new().get(url).send().await.unwrap();
        let status = response.status();
        let body = response.json().await.unwrap_or(serde_json::Value::Null);
        (status, body)
    }

    async fn test_provider(status: Arc<AtomicU16>, calls: Arc<AtomicUsize>) -> String {
        let upstream = mock_kakao(status, calls).await;
        let cfg = Arc::new(Config {
            auth_token: None,
            kakao_rest_api_key: "test-key".into(),
            user_agent: "test-agent".into(),
            requests_per_second: 2,
            max_inflight_requests: 2,
        });
        let client = Arc::new(Client::with_base_url(
            "test-key",
            "test-agent",
            &upstream,
            0,
            2,
        ));
        serve(routes(cfg, client)).await
    }

    #[tokio::test]
    async fn search_uses_local_upstream_and_maps_response() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = test_provider(Arc::new(AtomicU16::new(0)), calls.clone()).await;
        let (status, body) = get_json(&format!(
            "{base}/search?query=%EB%AF%B8%EC%9B%80%EB%B0%9B%EC%9D%84%20%EC%9A%A9%EA%B8%B0"
        ))
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!body["matches"].as_array().unwrap().is_empty());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "provider must call the local mock HTTP server"
        );
    }

    #[tokio::test]
    async fn lookup_returns_cached_search_result_after_mock_search() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = test_provider(Arc::new(AtomicU16::new(0)), calls.clone()).await;
        let (_, search_body) = get_json(&format!("{base}/search?query=book")).await;
        let id = search_body["matches"][0]["providerId"].as_str().unwrap();
        let (status, book) = get_json(&format!("{base}/book/{id}")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(book["providerId"], id);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "lookup is served from the provider cache after the mock-backed search"
        );
    }

    #[tokio::test]
    async fn upstream_error_is_mapped_to_bad_gateway() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = test_provider(Arc::new(AtomicU16::new(500)), calls.clone()).await;
        let (status, _) = get_json(&format!("{base}/search?query=test")).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn provider_info_exposes_contract_and_rate_limit() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = test_provider(Arc::new(AtomicU16::new(0)), calls.clone()).await;
        let (status, body) = get_json(&format!("{base}/provider/info")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["key"], "kakao");
        assert_eq!(body["rateLimit"]["minIntervalMs"], 500);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn captured_fixtures_remain_valid() {
        let isbn: crate::mapping::KakaoResponse = serde_json::from_str(ISBN_SUCCESS).unwrap();
        let title: crate::mapping::KakaoResponse = serde_json::from_str(TITLE_SUCCESS).unwrap();
        assert!(!isbn.documents.is_empty());
        assert!(!title.documents.is_empty());
    }
}
