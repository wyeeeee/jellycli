use std::sync::Arc;
use tokio::sync::RwLock;
use anyhow::{Result, Context};
use tracing::{info, warn, error, debug};
use uuid::Uuid;

use crate::auth::CredentialManager;
use crate::client::GeminiApiClient;
use crate::models::{
    OpenAIChatCompletionRequest, OpenAIChatCompletionResponse, 
    OpenAIChatCompletionStreamResponse, OpenAIChatCompletionStreamChoice,
    OpenAIDelta, ErrorResponse, ApiError
};
use crate::utils::{openai_to_gemini_request, gemini_to_openai_response, gemini_stream_to_openai_stream};

use axum::{
    response::{Response, Json, Sse, IntoResponse},
    http::StatusCode,
};
use axum::response::sse::{Event, KeepAlive};
use tokio_stream::{StreamExt, wrappers::ReceiverStream};

pub struct GeminiCliService {
    credential_manager: Arc<RwLock<CredentialManager>>,
    gemini_client: GeminiApiClient,
}

impl GeminiCliService {
    pub fn new(
        credential_manager: CredentialManager,
        code_assist_endpoint: String,
    ) -> Self {
        Self {
            credential_manager: Arc::new(RwLock::new(credential_manager)),
            gemini_client: GeminiApiClient::new(code_assist_endpoint),
        }
    }

    pub async fn initialize(&self) -> Result<()> {
        let mut manager = self.credential_manager.write().await;
        manager.initialize().await
            .context("Failed to initialize credential manager")?;
        
        // Try to get initial credentials and onboard
        if let Ok(Some((creds, project_id))) = manager.get_current_credentials().await {
            if let Some(project_id) = project_id {
                if let Err(e) = self.gemini_client.onboard_user(&creds, &project_id).await {
                    warn!("Initial onboarding failed: {}", e);
                } else {
                    info!("Successfully onboarded with project ID: {}", project_id);
                }
            }
        } else {
            warn!("No credentials available on startup - service will return errors until credentials are added via OAuth");
        }

        info!("GeminiCli service initialized successfully");
        Ok(())
    }

    pub async fn chat_completion(
        &self,
        mut request: OpenAIChatCompletionRequest,
    ) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
        // Handle health check
        if request.is_health_check() {
            let health_response = serde_json::json!({
                "choices": [{
                    "delta": {
                        "role": "assistant",
                        "content": "公益站正常工作中"
                    }
                }]
            });
            return Ok(Json(health_response).into_response());
        }

        // Process request
        request.limit_max_tokens();
        request.filter_empty_messages();

        let _original_model = request.model.clone();
        let real_model = request.get_real_model();
        let is_fake_streaming = request.is_fake_streaming();

        // Set real model for processing
        request.model = real_model.clone();

        // Handle fake streaming
        if is_fake_streaming {
            request.stream = false;
            return self.handle_fake_streaming(request).await;
        }

        // Prepare request
        let (gemini_request, creds, project_id, current_file_path) = 
            match self.prepare_request(&request).await {
                Ok(data) => data,
                Err(e) => {
                    error!("Failed to prepare request: {}", e);
                    return Err(self.create_error_response(
                        &format!("Request preparation failed: {}", e),
                        "invalid_request_error",
                        400,
                    ));
                }
            };

        // Handle streaming vs non-streaming
        if request.stream {
            self.handle_streaming_request(gemini_request, &real_model, &creds, &project_id, &current_file_path).await
        } else {
            self.handle_non_streaming_request(gemini_request, &real_model, &creds, &project_id, &current_file_path).await
        }
    }

    async fn prepare_request(
        &self,
        request: &OpenAIChatCompletionRequest,
    ) -> Result<(crate::models::GeminiRequest, crate::auth::GoogleCredentials, Option<String>, Option<String>)> {
        // Increment call count and get credentials
        let mut manager = self.credential_manager.write().await;
        manager.increment_call_count();

        let (mut creds, project_id) = manager.get_current_credentials().await?
            .ok_or_else(|| anyhow::anyhow!("No credentials available - please configure OAuth credentials via /auth endpoint"))?;

        let current_file_path = manager.get_current_file_path()
            .map(|p| p.to_string_lossy().to_string());

        // Refresh credentials if needed
        manager.refresh_credentials(&mut creds).await?;

        // Onboard user if needed
        if let Some(ref project_id) = project_id {
            self.gemini_client.onboard_user(&creds, project_id).await?;
        }

        // Convert request to Gemini format
        let gemini_request = openai_to_gemini_request(request);

        Ok((gemini_request, creds, project_id, current_file_path))
    }

    async fn handle_streaming_request(
        &self,
        gemini_request: crate::models::GeminiRequest,
        model: &str,
        creds: &crate::auth::GoogleCredentials,
        project_id: &Option<String>,
        current_file_path: &Option<String>,
    ) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
        let response_id = format!("chatcmpl-{}", Uuid::new_v4());
        let token_suffix = creds.access_token.as_ref()
            .map(|token| {
                let len = token.len();
                if len > 8 {
                    format!("...{}", &token[len-8..])
                } else {
                    format!("...{}", token)
                }
            })
            .unwrap_or_else(|| "...unknown".to_string());
        
        info!("🚀 Starting streaming request - Model: {}, Token: {}, RequestID: {}", 
              model, token_suffix, response_id);
        
        let project_id_str = project_id.as_ref().ok_or_else(|| {
            error!("No project ID available");
            self.create_error_response("No project ID available", "invalid_request_error", 400)
        }).map_err(|e| e)?;
        
        match self.gemini_client.send_streaming_request(&gemini_request, model, creds, project_id_str).await {
            Ok(mut stream) => {
                let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, axum::Error>>(100);
                let response_id_clone = response_id.clone();
                let model_clone = model.to_string();
                let current_file_clone = current_file_path.clone();
                let credential_manager = Arc::clone(&self.credential_manager);

                tokio::spawn(async move {
                    let mut has_error = false;

                    while let Some(result) = stream.next().await {
                        match result {
                            Ok(chunk) => {
                                let openai_chunk = gemini_stream_to_openai_stream(
                                    &chunk, 
                                    &model_clone, 
                                    &response_id_clone
                                );
                                
                                let event_data = serde_json::to_string(&openai_chunk)
                                    .unwrap_or_else(|_| "{}".to_string());
                                
                                if tx.send(Ok(Event::default().data(event_data))).await.is_err() {
                                    break;
                                }
                            },
                            Err(e) => {
                                error!("Streaming error: {}", e);
                                has_error = true;
                                
                                let error_chunk = serde_json::json!({
                                    "error": {
                                        "message": e.to_string(),
                                        "type": "api_error",
                                        "code": 500
                                    }
                                });
                                
                                let _ = tx.send(Ok(Event::default().data(error_chunk.to_string()))).await;
                                break;
                            }
                        }
                    }

                    // Send [DONE] event
                    let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;

                    // Record success/error and log completion
                    if let Some(file_path) = current_file_clone {
                        let mut manager = credential_manager.write().await;
                        if has_error {
                            let _ = manager.record_error(&file_path, 500).await;
                            info!("❌ Streaming request completed with error - RequestID: {}", response_id_clone);
                        } else {
                            let _ = manager.record_success(&file_path).await;
                            info!("✅ Streaming request completed successfully - RequestID: {}", response_id_clone);
                        }
                    } else {
                        if has_error {
                            info!("❌ Streaming request completed with error - RequestID: {}", response_id_clone);
                        } else {
                            info!("✅ Streaming request completed successfully - RequestID: {}", response_id_clone);
                        }
                    }
                });

                let stream = ReceiverStream::new(rx);
                Ok(Sse::new(stream)
                    .keep_alive(KeepAlive::default())
                    .into_response())
            },
            Err(e) => {
                error!("Failed to create streaming request: {}", e);
                info!("❌ Streaming request failed to start - Model: {}, Token: {}, Error: {}", 
                      model, token_suffix, e);
                
                // Record error if we have file path
                if let Some(file_path) = current_file_path {
                    let mut manager = self.credential_manager.write().await;
                    let _ = manager.record_error(file_path, 500).await;
                }

                Err(self.create_error_response(
                    &format!("Streaming request failed: {}", e),
                    "api_error",
                    500,
                ))
            }
        }
    }

    async fn handle_non_streaming_request(
        &self,
        gemini_request: crate::models::GeminiRequest,
        model: &str,
        creds: &crate::auth::GoogleCredentials,
        project_id: &Option<String>,
        current_file_path: &Option<String>,
    ) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
        let token_suffix = creds.access_token.as_ref()
            .map(|token| {
                let len = token.len();
                if len > 8 {
                    format!("...{}", &token[len-8..])
                } else {
                    format!("...{}", token)
                }
            })
            .unwrap_or_else(|| "...unknown".to_string());
        let request_id = format!("req-{}", Uuid::new_v4());
        
        info!("🚀 Starting non-streaming request - Model: {}, Token: {}, RequestID: {}", 
              model, token_suffix, request_id);
        
        let project_id_str = project_id.as_ref().ok_or_else(|| {
            error!("No project ID available");
            self.create_error_response("No project ID available", "invalid_request_error", 400)
        }).map_err(|e| e)?;
        
        match self.gemini_client.send_non_streaming_request(&gemini_request, model, creds, project_id_str).await {
            Ok(gemini_response) => {
                // Record success
                if let Some(file_path) = current_file_path {
                    let mut manager = self.credential_manager.write().await;
                    let _ = manager.record_success(file_path).await;
                }

                info!("✅ Non-streaming request completed successfully - RequestID: {}", request_id);
                let openai_response = gemini_to_openai_response(&gemini_response, model);
                Ok(Json(openai_response).into_response())
            },
            Err(e) => {
                error!("Non-streaming request failed: {}", e);
                info!("❌ Non-streaming request failed - RequestID: {}, Error: {}", request_id, e);
                
                // Record error if we have file path
                if let Some(file_path) = current_file_path {
                    let mut manager = self.credential_manager.write().await;
                    let _ = manager.record_error(file_path, 500).await;
                }

                Err(self.create_error_response(
                    &format!("Request failed: {}", e),
                    "api_error",
                    500,
                ))
            }
        }
    }

    async fn handle_fake_streaming(
        &self,
        mut request: OpenAIChatCompletionRequest,
    ) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
        let request_id = format!("fake-{}", Uuid::new_v4());
        
        // Get token for logging from current credentials
        let token_suffix = if let Ok(Some((creds, _))) = {
            let mut manager = self.credential_manager.write().await;
            manager.get_current_credentials().await
        } {
            creds.access_token.as_ref()
                .map(|token| {
                    let len = token.len();
                    if len > 8 {
                        format!("...{}", &token[len-8..])
                    } else {
                        format!("...{}", token)
                    }
                })
                .unwrap_or_else(|| "...unknown".to_string())
        } else {
            "...unknown".to_string()
        };
        
        info!("🚀 Starting fake streaming request - Model: {}, Token: {}, RequestID: {}", 
              request.model, token_suffix, request_id);
        
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, axum::Error>>(100);
        let service = self.clone();

        tokio::spawn(async move {
            // Send initial heartbeat
            let heartbeat = OpenAIChatCompletionStreamResponse {
                id: format!("chatcmpl-{}", Uuid::new_v4()),
                object: "chat.completion.chunk".to_string(),
                created: chrono::Utc::now().timestamp(),
                model: request.model.clone(),
                choices: vec![OpenAIChatCompletionStreamChoice {
                    index: 0,
                    delta: OpenAIDelta {
                        role: Some("assistant".to_string()),
                        content: Some("".to_string()),
                        reasoning_content: None,
                    },
                    finish_reason: None,
                }],
            };

            let heartbeat_data = serde_json::to_string(&heartbeat).unwrap_or_default();
            let _ = tx.send(Ok(Event::default().data(heartbeat_data))).await;

            // Process non-streaming request
            match service.handle_non_streaming_request_internal(request).await {
                Ok(response) => {
                    // Extract content and send as stream chunk
                    if let Ok(response_json) = serde_json::to_value(&response) {
                        if let Some(choices) = response_json.get("choices").and_then(|c| c.as_array()) {
                            if let Some(first_choice) = choices.first() {
                                if let Some(message) = first_choice.get("message") {
                                    if let Some(content) = message.get("content").and_then(|c| c.as_str()) {
                                        let content_chunk = OpenAIChatCompletionStreamResponse {
                                            id: format!("chatcmpl-{}", Uuid::new_v4()),
                                            object: "chat.completion.chunk".to_string(),
                                            created: chrono::Utc::now().timestamp(),
                                            model: response.model.clone(),
                                            choices: vec![OpenAIChatCompletionStreamChoice {
                                                index: 0,
                                                delta: OpenAIDelta {
                                                    role: None,
                                                    content: Some(content.to_string()),
                                                    reasoning_content: None,
                                                },
                                                finish_reason: Some("stop".to_string()),
                                            }],
                                        };

                                        let chunk_data = serde_json::to_string(&content_chunk).unwrap_or_default();
                                        let _ = tx.send(Ok(Event::default().data(chunk_data))).await;
                                    }
                                }
                            }
                        }
                    }
                },
                Err(e) => {
                    let error_chunk = serde_json::json!({
                        "error": {
                            "message": e.to_string(),
                            "type": "api_error",
                            "code": 500
                        }
                    });
                    let _ = tx.send(Ok(Event::default().data(error_chunk.to_string()))).await;
                    info!("❌ Fake streaming request failed - RequestID: {}, Error: {}", request_id, e);
                    return;
                }
            }

            // Send [DONE]
            let _ = tx.send(Ok(Event::default().data("[DONE]"))).await;
            info!("✅ Fake streaming request completed successfully - RequestID: {}", request_id);
        });

        let stream = ReceiverStream::new(rx);
        Ok(Sse::new(stream)
            .keep_alive(KeepAlive::default())
            .into_response())
    }

    async fn handle_non_streaming_request_internal(
        &self,
        request: OpenAIChatCompletionRequest,
    ) -> Result<OpenAIChatCompletionResponse> {
        let (gemini_request, creds, project_id, current_file_path) = self.prepare_request(&request).await?;
        let token_suffix = creds.access_token.as_ref()
            .map(|token| {
                let len = token.len();
                if len > 8 {
                    format!("...{}", &token[len-8..])
                } else {
                    format!("...{}", token)
                }
            })
            .unwrap_or_else(|| "...unknown".to_string());
        
        let project_id_str = project_id.as_ref().ok_or_else(|| anyhow::anyhow!("No project ID available"))?;
        
        debug!("Internal non-streaming request - Model: {}, Token: {}", request.model, token_suffix);
        
        let gemini_response = self.gemini_client
            .send_non_streaming_request(&gemini_request, &request.model, &creds, project_id_str)
            .await?;

        // Record success
        if let Some(file_path) = current_file_path {
            let mut manager = self.credential_manager.write().await;
            let _ = manager.record_success(&file_path).await;
        }

        Ok(gemini_to_openai_response(&gemini_response, &request.model))
    }

    fn create_error_response(
        &self,
        message: &str,
        error_type: &str,
        status_code: u16,
    ) -> (StatusCode, Json<ErrorResponse>) {
        (
            StatusCode::from_u16(status_code).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            Json(ErrorResponse {
                error: ApiError {
                    message: message.to_string(),
                    error_type: error_type.to_string(),
                    code: status_code,
                },
            }),
        )
    }
}

impl Clone for GeminiCliService {
    fn clone(&self) -> Self {
        Self {
            credential_manager: Arc::clone(&self.credential_manager),
            gemini_client: GeminiApiClient::new(self.gemini_client.code_assist_endpoint.clone()),
        }
    }
}