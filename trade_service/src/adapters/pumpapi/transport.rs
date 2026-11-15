use anyhow::{bail, Context};
use serde_json::Value;
use std::time::Duration;

use super::response::MAX_RESPONSE_BYTES;
use crate::{config::PumpApiConfig, sanitize::redact_urls};

pub(crate) async fn fetch_transaction(
    config: &PumpApiConfig,
    request: &Value,
) -> anyhow::Result<Vec<u8>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(config.timeout_ms))
        .build()
        .context("build PumpAPI http client")?;
    let response = client
        .post(&config.url)
        .json(request)
        .send()
        .await
        .map_err(|error| {
            anyhow::anyhow!(
                "PumpAPI request failed: {}",
                redact_urls(&error.to_string())
            )
        })?;
    read_transaction_response(response).await
}

async fn read_transaction_response(response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    let status = response.status();
    let body = read_body(response).await?;
    if !status.is_success() {
        bail!(
            "PumpAPI returned HTTP {status}: {}",
            redact_urls(String::from_utf8_lossy(&body).trim())
        );
    }
    Ok(body)
}

async fn read_body(mut response: reqwest::Response) -> anyhow::Result<Vec<u8>> {
    let mut body = BoundedBody::new(response.content_length())?;
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        anyhow::anyhow!(
            "read PumpAPI transaction response: {}",
            redact_urls(&error.to_string())
        )
    })? {
        body.push(&chunk)?;
    }
    Ok(body.bytes)
}

struct BoundedBody {
    bytes: Vec<u8>,
}

impl BoundedBody {
    fn new(content_length: Option<u64>) -> anyhow::Result<Self> {
        if content_length.is_some_and(|length| length > MAX_RESPONSE_BYTES as u64) {
            bail!("PumpAPI transaction response exceeds {MAX_RESPONSE_BYTES} bytes");
        }
        Ok(Self { bytes: Vec::new() })
    }

    fn push(&mut self, chunk: &[u8]) -> anyhow::Result<()> {
        if chunk.len() > MAX_RESPONSE_BYTES - self.bytes.len() {
            bail!("PumpAPI transaction response exceeds {MAX_RESPONSE_BYTES} bytes");
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pumpapi_h1_body_chunks_enforce_actual_size_without_partial_overflow() {
        for advertised in [None, Some(0), Some(1), Some(MAX_RESPONSE_BYTES as u64)] {
            let mut body = BoundedBody::new(advertised).unwrap();
            for _ in 0..16 {
                body.push(&[7; 1024]).unwrap();
            }
            assert_eq!(body.bytes.len(), MAX_RESPONSE_BYTES);
            assert!(body.push(&[8]).is_err());
            assert_eq!(body.bytes, vec![7; MAX_RESPONSE_BYTES]);
            body.push(&[]).unwrap();
        }
        assert!(BoundedBody::new(Some(MAX_RESPONSE_BYTES as u64 + 1)).is_err());
        assert!(BoundedBody::new(Some(u64::MAX)).is_err());
        let mut body = BoundedBody::new(None).unwrap();
        assert!(body.push(&vec![0; MAX_RESPONSE_BYTES + 1]).is_err());
        assert!(body.bytes.is_empty());
    }

    #[tokio::test]
    async fn pumpapi_h1_reqwest_body_read_is_bounded_without_sockets() {
        for size in [0, 1, MAX_RESPONSE_BYTES, MAX_RESPONSE_BYTES + 1] {
            let response = axum::http::Response::builder().body(vec![7; size]).unwrap();
            let result = read_body(reqwest::Response::from(response)).await;
            if size > MAX_RESPONSE_BYTES {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), vec![7; size]);
            }
        }
    }

    #[tokio::test]
    async fn pumpapi_h1_http_error_body_redacts_urls_and_respects_cap() {
        for status in [200, 400, 429, 502] {
            let bytes = b"failure at https://fixture.invalid/private-token?key=secret".to_vec();
            let response = axum::http::Response::builder()
                .status(status)
                .body(bytes.clone())
                .unwrap();
            let result = read_transaction_response(reqwest::Response::from(response)).await;
            if status == 200 {
                assert_eq!(result.unwrap(), bytes);
            } else {
                let error = format!("{:#}", result.unwrap_err());
                assert!(error.contains(&format!("HTTP {status}")));
                assert!(error.contains("<redacted-url>"));
                assert!(!error.contains("private-token"));
                assert!(!error.contains("secret"));
            }
        }
        for status in [200, 502] {
            let response = axum::http::Response::builder()
                .status(status)
                .body(vec![b'x'; MAX_RESPONSE_BYTES + 1])
                .unwrap();
            let error = read_transaction_response(reqwest::Response::from(response))
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "PumpAPI transaction response exceeds 16384 bytes"
            );
        }
    }
}
