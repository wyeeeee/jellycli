use anyhow::{Context, Result};
use reqwest::{Client, Response, header::HeaderMap};
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::{Stream, StreamExt};
use tracing::{debug, info, warn};

use crate::auth::GoogleCredentials;
use crate::models::{GeminiRequest, GeminiResponse, GeminiStreamChunk};
use crate::utils::{get_client_metadata, get_user_agent};

pub struct GeminiApiClient {
    http_client: Client,
    pub code_assist_endpoint: String,
    config: Arc<crate::utils::AppConfig>,
}

impl GeminiApiClient {
    pub fn new(code_assist_endpoint: String, config: Arc<crate::utils::AppConfig>) -> Self {
        Self {
            http_client: Client::new(),
            code_assist_endpoint,
            config,
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

        // Print detailed debug info if enabled
        if self.config.debug_api {
            info!("🚀 [DEBUG] Gemini API Request:");
            info!("🚀 [DEBUG] URL: {}", url);
            info!("🚀 [DEBUG] Headers: {:?}", headers);
            info!("🚀 [DEBUG] Payload: {}", serde_json::to_string_pretty(&final_payload).unwrap_or_default());
        }

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

        let stream = response.bytes_stream();
        let debug_enabled = self.config.debug_api;
        let chunk_stream = stream.map(move |result| {
            match result {
                Ok(bytes) => {
                    let text = String::from_utf8_lossy(&bytes);

                    // Debug output for streaming chunks
                    if debug_enabled {
                        info!("🌊 [DEBUG] Streaming chunk: {}", text);
                    }

                    // Parse SSE format
                    if let Some(stripped) = text.strip_prefix("data: ") {
                        let json_str = stripped.trim();
                        if json_str == "[DONE]" {
                            return Err(anyhow::anyhow!("Stream complete"));
                        }

                        // First try to parse as wrapped response
                        match serde_json::from_str::<serde_json::Value>(json_str) {
                            Ok(value) => {
                                if let Some(response_data) = value.get("response") {
                                    // Try to parse the inner response as GeminiStreamChunk
                                    match serde_json::from_value::<GeminiStreamChunk>(
                                        response_data.clone(),
                                    ) {
                                        Ok(chunk) => Ok(chunk),
                                        Err(e) => {
                                            warn!("Failed to parse inner response: {}", e);
                                            // Create a basic chunk for non-standard responses
                                            Ok(GeminiStreamChunk {
                                                candidates: vec![],
                                                usage_metadata: None,
                                            })
                                        }
                                    }
                                } else {
                                    // Try to parse directly as GeminiStreamChunk
                                    match serde_json::from_value::<GeminiStreamChunk>(value) {
                                        Ok(chunk) => Ok(chunk),
                                        Err(e) => {
                                            warn!("Failed to parse stream chunk: {}", e);
                                            // Create a basic chunk for non-standard responses
                                            Ok(GeminiStreamChunk {
                                                candidates: vec![],
                                                usage_metadata: None,
                                            })
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Failed to parse JSON: {}", e);
                                Err(anyhow::anyhow!("Parse error: {}", e))
                            }
                        }
                    } else {
                        // Try to parse as JSON directly
                        match serde_json::from_str::<serde_json::Value>(&text) {
                            Ok(value) => {
                                if let Some(response_data) = value.get("response") {
                                    match serde_json::from_value::<GeminiStreamChunk>(
                                        response_data.clone(),
                                    ) {
                                        Ok(chunk) => Ok(chunk),
                                        Err(_) => Ok(GeminiStreamChunk {
                                            candidates: vec![],
                                            usage_metadata: None,
                                        }),
                                    }
                                } else {
                                    match serde_json::from_value::<GeminiStreamChunk>(value) {
                                        Ok(chunk) => Ok(chunk),
                                        Err(_) => Ok(GeminiStreamChunk {
                                            candidates: vec![],
                                            usage_metadata: None,
                                        }),
                                    }
                                }
                            }
                            Err(e) => {
                                debug!("Skipping non-JSON chunk: {}", text);
                                Err(anyhow::anyhow!("Non-JSON chunk: {}", e))
                            }
                        }
                    }
                }
                Err(e) => Err(anyhow::anyhow!("Stream error: {}", e)),
            }
        });

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

        let status = response.status();
        let headers = response.headers().clone();

        if !status.is_success() {
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

        // Print detailed debug info if enabled
        if self.config.debug_api {

            info!("🔍 [DEBUG] Raw Gemini API Response:");
            info!("🔍 [DEBUG] Status: {}", status);
            info!("🔍 [DEBUG] Headers: {:?}", headers);
            info!("🔍 [DEBUG] Response Body: {}", response_text);

            // Try to parse and print structured data
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&response_text) {
                info!("🔍 [DEBUG] Parsed JSON Response:");
                info!("🔍 [DEBUG] {}", serde_json::to_string_pretty(&parsed).unwrap_or_else(|_| "Failed to pretty print".to_string()));

                // Check for image data
                if let Some(candidates) = parsed.get("candidates").and_then(|c| c.as_array()) {
                    for (i, candidate) in candidates.iter().enumerate() {
                        info!("🔍 [DEBUG] Candidate {}: {}", i, serde_json::to_string_pretty(candidate).unwrap_or_default());

                        if let Some(content) = candidate.get("content") {
                            if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                                for (j, part) in parts.iter().enumerate() {
                                    info!("🔍 [DEBUG]   Part {}: {}", j, serde_json::to_string_pretty(part).unwrap_or_default());

                                    // Check if this part contains image data
                                    if part.get("inlineData").is_some() {
                                        info!("🖼️ [DEBUG]   🎯 Found image data in part {}!", j);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

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
