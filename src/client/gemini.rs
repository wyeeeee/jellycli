use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::{Client, Response, header::HeaderMap};
use serde_json::Value;
use std::pin::Pin;
use tokio_stream::Stream;
use tracing::{debug, info, warn};

use crate::auth::GoogleCredentials;
use crate::models::{GeminiRequest, GeminiResponse, GeminiStreamChunk};
use crate::utils::{get_client_metadata, get_user_agent};

pub struct GeminiApiClient {
    http_client: Client,
    pub code_assist_endpoint: String,
}

impl GeminiApiClient {
    pub fn new(code_assist_endpoint: String) -> Self {
        Self {
            http_client: Client::new(),
            code_assist_endpoint,
        }
    }

    pub async fn onboard_user(&self, creds: &GoogleCredentials, project_id: &str) -> Result<()> {
        if let Some(access_token) = &creds.access_token {
            let headers = self.create_headers(access_token)?;

            // Check if user is already onboarded
            let load_assist_payload = serde_json::json!({
                "cloudaicompanionProject": project_id,
                "metadata": get_client_metadata(project_id),
            });

            let load_response = self
                .http_client
                .post(format!(
                    "{}/v1internal:loadCodeAssist",
                    self.code_assist_endpoint
                ))
                .headers(headers.clone())
                .json(&load_assist_payload)
                .send()
                .await
                .context("Failed to load code assist")?;

            if !load_response.status().is_success() {
                return Err(anyhow::anyhow!(
                    "Load code assist failed with status: {}",
                    load_response.status()
                ));
            }

            let load_data: Value = load_response
                .json()
                .await
                .context("Failed to parse load response")?;

            // Check if already onboarded
            if load_data.get("currentTier").is_some() {
                debug!("User already onboarded");
                return Ok(());
            }

            // Determine tier
            let tier = if let Some(allowed_tiers) =
                load_data.get("allowedTiers").and_then(|t| t.as_array())
            {
                allowed_tiers
                    .iter()
                    .find(|tier| {
                        tier.get("isDefault")
                            .and_then(|d| d.as_bool())
                            .unwrap_or(false)
                    })
                    .cloned()
                    .unwrap_or_else(|| {
                        serde_json::json!({
                            "name": "",
                            "description": "",
                            "id": "legacy-tier",
                            "userDefinedCloudaicompanionProject": true,
                        })
                    })
            } else {
                serde_json::json!({
                    "name": "",
                    "description": "",
                    "id": "legacy-tier",
                    "userDefinedCloudaicompanionProject": true,
                })
            };

            // Onboard user
            let onboard_payload = serde_json::json!({
                "tierId": tier.get("id"),
                "cloudaicompanionProject": project_id,
                "metadata": get_client_metadata(project_id),
            });

            loop {
                let onboard_response = self
                    .http_client
                    .post(format!(
                        "{}/v1internal:onboardUser",
                        self.code_assist_endpoint
                    ))
                    .headers(headers.clone())
                    .json(&onboard_payload)
                    .send()
                    .await
                    .context("Failed to onboard user")?;

                if !onboard_response.status().is_success() {
                    let error_text = onboard_response.text().await.unwrap_or_default();
                    return Err(anyhow::anyhow!("User onboarding failed: {}", error_text));
                }

                let lro_data: Value = onboard_response
                    .json()
                    .await
                    .context("Failed to parse onboard response")?;

                if lro_data
                    .get("done")
                    .and_then(|d| d.as_bool())
                    .unwrap_or(false)
                {
                    info!("User onboarding completed successfully");
                    break;
                }

                // Wait before checking again
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        }

        Ok(())
    }

    pub async fn send_request(
        &self,
        request: &GeminiRequest,
        model: &str,
        creds: &GoogleCredentials,
        project_id: &str,
        stream: bool,
    ) -> Result<Response> {
        let access_token = creds
            .access_token
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No access token available"))?;

        let headers = self.create_headers(access_token)?;

        let url = if stream {
            format!(
                "{}/v1internal:streamGenerateContent?alt=sse",
                self.code_assist_endpoint
            )
        } else {
            format!("{}/v1internal:generateContent", self.code_assist_endpoint)
        };

        // Construct payload in the same format as Python version
        let final_payload = serde_json::json!({
            "model": model,
            "project": project_id,
            "request": request
        });

        debug!("Sending request to: {}", url);
        debug!(
            "Request payload: {}",
            serde_json::to_string_pretty(&final_payload)?
        );

        let response = self
            .http_client
            .post(&url)
            .headers(headers)
            .json(&final_payload)
            .send()
            .await
            .context("Failed to send Gemini request")?;

        Ok(response)
    }

    pub async fn send_streaming_request(
        &self,
        request: &GeminiRequest,
        model: &str,
        creds: &GoogleCredentials,
        project_id: &str,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<GeminiStreamChunk>> + Send>>> {
        let response = self
            .send_request(request, model, creds, project_id, true)
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Streaming request failed {}: {}",
                status,
                error_text
            ));
        }

        let byte_stream = response.bytes_stream();

        // 使用 unfold 来维护跨数据块的缓冲区
        let chunk_stream = futures::stream::unfold(
            (String::new(), byte_stream),
            |(mut buffer, mut stream)| async move {
                loop {
                    // 尝试从缓冲区解析完整的行
                    if let Some(end_pos) = buffer.find('\n') {
                        let line = buffer[..end_pos].trim().to_string();
                        buffer = buffer[end_pos + 1..].to_string();

                        if line.is_empty() {
                            continue;
                        }

                        // 解析 SSE 格式的数据行
                        let json_str = if let Some(stripped) = line.strip_prefix("data:") {
                            stripped.trim()
                        } else {
                            line.trim()
                        };

                        if json_str.is_empty() {
                            continue;
                        }

                        if json_str == "[DONE]" {
                            return None;
                        }

                        // 尝试解析完整的 JSON
                        match serde_json::from_str::<serde_json::Value>(json_str) {
                            Ok(value) => {
                                let chunk_result = if let Some(response_data) = value.get("response") {
                                    serde_json::from_value::<GeminiStreamChunk>(response_data.clone())
                                } else {
                                    serde_json::from_value::<GeminiStreamChunk>(value)
                                };

                                let result = match chunk_result {
                                    Ok(chunk) => Ok(chunk),
                                    Err(e) => {
                                        warn!("Failed to parse chunk: {}", e);
                                        // 对于解析失败的块,返回空块而不是错误
                                        Ok(GeminiStreamChunk {
                                            candidates: vec![],
                                            usage_metadata: None,
                                        })
                                    }
                                };

                                return Some((result, (buffer, stream)));
                            }
                            Err(e) => {
                                // JSON 解析失败,可能是不完整的 JSON
                                // 将这行放回缓冲区等待更多数据
                                debug!("JSON parse failed, buffering: {} (error: {})", json_str, e);
                                buffer = format!("{}\n{}", line, buffer);
                                // 继续读取更多数据
                            }
                        }
                    }

                    // 需要更多数据,从流中读取下一个块
                    match stream.next().await {
                        Some(Ok(bytes)) => {
                            let text = String::from_utf8_lossy(&bytes);
                            buffer.push_str(&text);
                            // 继续循环尝试解析
                        }
                        Some(Err(e)) => {
                            return Some((
                                Err(anyhow::anyhow!("Stream error: {}", e)),
                                (buffer, stream),
                            ));
                        }
                        None => {
                            // 流结束
                            if !buffer.is_empty() {
                                debug!("Stream ended with remaining buffer: {}", buffer);
                            }
                            return None;
                        }
                    }
                }
            },
        );

        Ok(Box::pin(chunk_stream))
    }

    pub async fn send_non_streaming_request(
        &self,
        request: &GeminiRequest,
        model: &str,
        creds: &GoogleCredentials,
        project_id: &str,
    ) -> Result<GeminiResponse> {
        let response = self
            .send_request(request, model, creds, project_id, false)
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "Non-streaming request failed {}: {}",
                status,
                error_text
            ));
        }

        let response_text = response
            .text()
            .await
            .context("Failed to get response text")?;

        debug!("Raw Gemini response: {}", response_text);

        // Try to parse as wrapped response first (like streaming does)
        let parsed_response: serde_json::Value =
            serde_json::from_str(&response_text).context("Failed to parse response as JSON")?;

        // Check if response is wrapped in a "response" field
        let gemini_response = if let Some(inner_response) = parsed_response.get("response") {
            serde_json::from_value::<GeminiResponse>(inner_response.clone())
                .context("Failed to parse wrapped Gemini response")?
        } else {
            serde_json::from_value::<GeminiResponse>(parsed_response)
                .context("Failed to parse direct Gemini response")?
        };

        Ok(gemini_response)
    }

    fn create_headers(&self, access_token: &str) -> Result<HeaderMap> {
        let mut headers = HeaderMap::new();

        headers.insert(
            "authorization",
            format!("Bearer {}", access_token)
                .parse()
                .context("Failed to create authorization header")?,
        );

        headers.insert(
            "content-type",
            "application/json"
                .parse()
                .context("Failed to create content-type header")?,
        );

        headers.insert(
            "user-agent",
            get_user_agent()
                .parse()
                .context("Failed to create user-agent header")?,
        );

        Ok(headers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{GeminiContent, GeminiPart};

    #[tokio::test]
    async fn test_gemini_client_creation() {
        let client = GeminiApiClient::new("https://test.example.com".to_string());
        assert_eq!(client.code_assist_endpoint, "https://test.example.com");
    }

    #[tokio::test]
    async fn test_headers_creation() {
        let client = GeminiApiClient::new("https://test.example.com".to_string());
        let headers = client.create_headers("test_token").unwrap();

        assert!(headers.contains_key("authorization"));
        assert!(headers.contains_key("content-type"));
        assert!(headers.contains_key("user-agent"));
    }
}
