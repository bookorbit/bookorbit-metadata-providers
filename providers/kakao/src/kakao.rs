use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Semaphore};

use reqwest::{header, StatusCode};
use serde::de::DeserializeOwned;

use crate::mapping::KakaoResponse;

pub const DEFAULT_API_URL: &str = "https://dapi.kakao.com/v3/search/book";
pub const DEFAULT_REQUESTS_PER_SECOND: u32 = 2;
pub const DEFAULT_MAX_INFLIGHT_REQUESTS: usize = 2;
const MAX_BODY_BYTES: usize = 8 << 20;

#[derive(Debug)]
pub enum UpstreamError {
    RateLimited { retry_after_seconds: u64 },
    Unauthorized,
    Failed(String),
}

pub struct Client {
    key: String,
    base_url: String,
    http: reqwest::Client,
    pacer: Arc<Pacer>,
    inflight: Arc<Semaphore>,
}

impl Client {
    // Kept for callers that use the default upstream configuration.
    #[allow(dead_code)]
    pub fn new(key: &str, user_agent: &str) -> Self {
        Self::with_base_url(
            key,
            user_agent,
            DEFAULT_API_URL,
            DEFAULT_REQUESTS_PER_SECOND,
            DEFAULT_MAX_INFLIGHT_REQUESTS,
        )
    }

    pub fn with_base_url(
        key: &str,
        user_agent: &str,
        base_url: &str,
        requests_per_second: u32,
        max_inflight_requests: usize,
    ) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::limited(3))
            .user_agent(user_agent)
            .build()
            .expect("could not build HTTP client");
        Self {
            key: key.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            http,
            pacer: Arc::new(Pacer::new(requests_per_second)),
            inflight: Arc::new(Semaphore::new(max_inflight_requests.max(1))),
        }
    }

    pub async fn search(
        &self,
        query: &str,
        target: Option<&str>,
        sort: Option<&str>,
        page: u32,
        size: u32,
    ) -> Result<KakaoResponse, UpstreamError> {
        let _permit = self
            .inflight
            .acquire()
            .await
            .expect("in-flight semaphore closed");
        self.pacer.wait().await;
        let page = page.to_string();
        let size = size.to_string();
        let mut request = self
            .http
            .get(&self.base_url)
            .header(header::AUTHORIZATION, format!("KakaoAK {}", self.key))
            .query(&[
                ("query", query),
                ("page", page.as_str()),
                ("size", size.as_str()),
            ]);
        if let Some(target) = target {
            request = request.query(&[("target", target)]);
        }
        if let Some(sort) = sort {
            request = request.query(&[("sort", sort)]);
        }
        let response = request
            .send()
            .await
            .map_err(|e| UpstreamError::Failed(e.to_string()))?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(UpstreamError::Unauthorized);
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            let retry_after_seconds = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v > 0)
                .unwrap_or(60);
            return Err(UpstreamError::RateLimited {
                retry_after_seconds,
            });
        }
        if !status.is_success() {
            return Err(UpstreamError::Failed(format!(
                "Kakao API answered {status}"
            )));
        }
        decode_limited(response).await
    }
}

async fn decode_limited<T: DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, UpstreamError> {
    let mut body = Vec::new();
    let mut stream = response;
    while let Some(chunk) = stream
        .chunk()
        .await
        .map_err(|e| UpstreamError::Failed(e.to_string()))?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(UpstreamError::Failed("Kakao response too large".into()));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body)
        .map_err(|e| UpstreamError::Failed(format!("Kakao response unreadable: {e}")))
}

/// Spaces upstream request starts according to the configured requests-per-second limit.
struct Pacer {
    interval: Duration,
    next: Mutex<Instant>,
}

impl Pacer {
    fn new(requests_per_second: u32) -> Self {
        let interval = if requests_per_second == 0 {
            Duration::ZERO
        } else {
            Duration::from_secs_f64(1.0 / requests_per_second as f64)
        };
        Self {
            interval,
            next: Mutex::new(Instant::now()),
        }
    }

    async fn wait(&self) {
        if self.interval.is_zero() {
            return;
        }
        let mut next = self.next.lock().await;
        let now = Instant::now();
        if *next > now {
            tokio::time::sleep(*next - now).await;
        }
        *next = Instant::now() + self.interval;
    }
}
