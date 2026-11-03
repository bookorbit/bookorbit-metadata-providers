use reqwest::{Client as HttpClient, StatusCode};
use serde::Deserialize;
use std::time::Duration;

use crate::mapping::{Yes24Envelope, Yes24Item};

const BASE_URL: &str = "https://apis.yes24.com/v1/goods";

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
}

impl Client {
    pub fn new(api_key: &str, user_agent: &str) -> Self {
        let http = HttpClient::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(20))
            .build()
            .expect("could not build HTTP client");
        Self {
            http,
            api_key: api_key.to_string(),
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
        let url = format!("{BASE_URL}/itemList");
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
        let url = format!("{BASE_URL}/itemDetail");
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
        let url = format!("{BASE_URL}/itemDetail");
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
