use reqwest::{Client as HttpClient, StatusCode};
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};

use crate::mapping::{Yes24Envelope, Yes24Item};

pub const DEFAULT_BASE_URL: &str = "https://apis.yes24.com/v1/goods";
pub const DEFAULT_REQUESTS_PER_SECOND: u32 = 2;
pub const DEFAULT_MAX_INFLIGHT_REQUESTS: usize = 2;

#[derive(Debug)]
pub enum UpstreamError {
    Unauthorized,
    RateLimited { retry_after_seconds: u64 },
    NotFound,
    Failed(String),
}

pub struct Client {
    http: HttpClient,
    api_key: String,
    base_url: String,
    pacer: Arc<Pacer>,
    inflight: Arc<Semaphore>,
}

impl Client {
    // Kept for callers that use the default upstream configuration.
    #[allow(dead_code)]
    pub fn new(api_key: &str, user_agent: &str) -> Self {
        Self::with_base_url(
            api_key,
            user_agent,
            DEFAULT_BASE_URL,
            DEFAULT_REQUESTS_PER_SECOND,
            DEFAULT_MAX_INFLIGHT_REQUESTS,
        )
    }

    pub fn with_base_url(
        api_key: &str,
        user_agent: &str,
        base_url: &str,
        requests_per_second: u32,
        max_inflight_requests: usize,
    ) -> Self {
        let http = HttpClient::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(20))
            .build()
            .expect("could not build HTTP client");
        Self {
            http,
            api_key: api_key.to_string(),
            base_url: base_url.trim_end_matches('/').to_string(),
            pacer: Arc::new(Pacer::new(requests_per_second)),
            inflight: Arc::new(Semaphore::new(max_inflight_requests.max(1))),
        }
    }

    pub async fn search(
        &self,
        query: &str,
        category: &str,
        sort: &str,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<Yes24Item>, UpstreamError> {
        let _permit = self
            .inflight
            .acquire()
            .await
            .expect("in-flight semaphore closed");
        self.pacer.wait().await;
        let url = format!("{}/itemList", self.base_url);
        let response = self
            .http
            .get(url)
            .header("X-Api-Key", &self.api_key)
            .query(&[
                ("query", query),
                ("category", category),
                ("sort", sort),
                ("page", &page.to_string()),
                ("pageSize", &page_size.to_string()),
                ("detail", "Y"),
            ])
            .send()
            .await
            .map_err(|e| UpstreamError::Failed(e.to_string()))?;
        let response = check_status(response).await?;
        let envelope: Yes24Envelope = response
            .json()
            .await
            .map_err(|e| UpstreamError::Failed(format!("invalid JSON response: {e}")))?;
        if !envelope.success {
            return Err(UpstreamError::Failed(format!(
                "YES24 API error {}: {}",
                envelope.error_code.unwrap_or_default(),
                envelope.message
            )));
        }
        Ok(envelope.data.items)
    }

    pub async fn detail_by_item_id(
        &self,
        item_id: u64,
    ) -> Result<Option<Yes24Item>, UpstreamError> {
        let _permit = self
            .inflight
            .acquire()
            .await
            .expect("in-flight semaphore closed");
        self.pacer.wait().await;
        let url = format!("{}/itemDetail", self.base_url);
        let response = self
            .http
            .get(url)
            .header("X-Api-Key", &self.api_key)
            .query(&[
                ("searchType", "ItemId"),
                ("query", &item_id.to_string()),
                ("detail", "Y"),
            ])
            .send()
            .await
            .map_err(|e| UpstreamError::Failed(e.to_string()))?;
        let response = match check_status(response).await {
            Ok(response) => response,
            Err(UpstreamError::NotFound) => return Ok(None),
            Err(err) => return Err(err),
        };
        let envelope: Yes24Envelope = response
            .json()
            .await
            .map_err(|e| UpstreamError::Failed(format!("invalid JSON response: {e}")))?;
        if !envelope.success {
            if envelope
                .error_code
                .as_deref()
                .is_some_and(|code| code == "GOODS_001" || code == "GOODS_002")
            {
                return Ok(None);
            }
            return Err(UpstreamError::Failed(format!(
                "YES24 API error {}: {}",
                envelope.error_code.unwrap_or_default(),
                envelope.message
            )));
        }
        Ok(envelope.data.items.into_iter().next())
    }

    pub async fn detail_by_isbn13(&self, isbn13: &str) -> Result<Option<Yes24Item>, UpstreamError> {
        let _permit = self
            .inflight
            .acquire()
            .await
            .expect("in-flight semaphore closed");
        self.pacer.wait().await;
        let url = format!("{}/itemDetail", self.base_url);
        let response = self
            .http
            .get(url)
            .header("X-Api-Key", &self.api_key)
            .query(&[("searchType", "ISBN13"), ("query", isbn13), ("detail", "Y")])
            .send()
            .await
            .map_err(|e| UpstreamError::Failed(e.to_string()))?;
        let response = match check_status(response).await {
            Ok(response) => response,
            Err(UpstreamError::NotFound) => return Ok(None),
            Err(err) => return Err(err),
        };
        let envelope: Yes24Envelope = response
            .json()
            .await
            .map_err(|e| UpstreamError::Failed(format!("invalid JSON response: {e}")))?;
        if !envelope.success {
            if envelope
                .error_code
                .as_deref()
                .is_some_and(|code| code == "GOODS_001" || code == "GOODS_002")
            {
                return Ok(None);
            }
            return Err(UpstreamError::Failed(format!(
                "YES24 API error {}: {}",
                envelope.error_code.unwrap_or_default(),
                envelope.message
            )));
        }
        Ok(envelope.data.items.into_iter().next())
    }
}

async fn check_status(response: reqwest::Response) -> Result<reqwest::Response, UpstreamError> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
        return Err(UpstreamError::Unauthorized);
    }
    if status == StatusCode::NOT_FOUND {
        return Err(UpstreamError::NotFound);
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        let retry_after_seconds = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(5);
        return Err(UpstreamError::RateLimited {
            retry_after_seconds,
        });
    }
    let body = response.text().await.unwrap_or_default();
    Err(UpstreamError::Failed(format!(
        "HTTP {status}: {}",
        body.chars().take(300).collect::<String>()
    )))
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct _ApiErrorShape {
    success: Option<bool>,
    message: Option<String>,
    error_code: Option<String>,
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
