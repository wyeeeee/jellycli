use crate::models::{
    OpenAIChatCompletionRequest, OpenAIChatCompletionResponse, OpenAIChatCompletionChoice,
    OpenAIChatCompletionStreamResponse, OpenAIChatCompletionStreamChoice,
    OpenAIChatMessage, OpenAIDelta,
    GeminiRequest, GeminiContent, GeminiPart, GeminiGenerationConfig,
    GeminiResponse, GeminiStreamChunk
};
use crate::utils::thinking_config::*;
use serde_json::Value;
use uuid::Uuid;
use chrono::Utc;

pub fn openai_to_gemini_request(openai_req: &OpenAIChatCompletionRequest) -> GeminiRequest {
    let contents = openai_req.messages.iter().map(|msg| {
        let role = match msg.role.as_str() {
            "system" => "user".to_string(),
            "assistant" => "model".to_string(),
            _ => msg.role.clone(),
        };

        let text = match &msg.content {
            Value::String(s) => s.clone(),
            Value::Array(arr) => {
                // Handle array of message parts
                arr.iter()
                    .filter_map(|item| {
                        if let Value::Object(obj) = item {
                            if let Some(Value::String(text)) = obj.get("text") {
                                Some(text.as_str())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            },
            _ => msg.content.to_string(),
        };

        GeminiContent {
            role,
            parts: vec![GeminiPart { text, thought: false }],
        }
    }).collect();

    let generation_config = if openai_req.temperature.is_some() 
        || openai_req.top_p.is_some() 
        || openai_req.max_tokens.is_some()
        || openai_req.stop.is_some() {
        Some(GeminiGenerationConfig {
            temperature: openai_req.temperature,
            top_p: openai_req.top_p,
            top_k: None,
            max_output_tokens: openai_req.max_tokens,
            stop_sequences: openai_req.stop.as_ref().and_then(|stop| {
                match stop {
                    Value::String(s) => Some(vec![s.clone()]),
                    Value::Array(arr) => Some(
                        arr.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    ),
                    _ => None,
                }
            }),
            thinking_config: None,
        })
    } else {
        None
    };

    let mut request = GeminiRequest {
        contents,
        generation_config,
        safety_settings: None,
    };
    
    // Add thinking configuration if needed
    if let Some(thinking_config) = get_thinking_config(&openai_req.model) {
        if let Some(ref mut gen_config) = request.generation_config {
            gen_config.thinking_config = Some(thinking_config);
        } else {
            request.generation_config = Some(GeminiGenerationConfig {
                temperature: None,
                top_p: None,
                top_k: None,
                max_output_tokens: None,
                stop_sequences: None,
                thinking_config: Some(thinking_config),
            });
        }
    }
    
    request
}

fn extract_content_and_reasoning(parts: &[GeminiPart]) -> (String, String) {
    let mut content = String::new();
    let mut reasoning_content = String::new();
    
    for part in parts {
        if !part.text.is_empty() {
            if part.thought {
                reasoning_content.push_str(&part.text);
            } else {
                content.push_str(&part.text);
            }
        }
    }
    
    (content, reasoning_content)
}

fn build_message_with_reasoning(role: &str, content: String, reasoning_content: String) -> OpenAIChatMessage {
    let mut message = OpenAIChatMessage {
        role: role.to_string(),
        content: Value::String(content),
        reasoning_content: None,
    };
    
    if !reasoning_content.is_empty() {
        message.reasoning_content = Some(reasoning_content);
    }
    
    message
}

pub fn gemini_to_openai_response(
    gemini_resp: &GeminiResponse, 
    model: &str
) -> OpenAIChatCompletionResponse {
    let id = format!("chatcmpl-{}", Uuid::new_v4());
    let created = Utc::now().timestamp();

    let choices = gemini_resp.candidates.iter().map(|candidate| {
        let role = if candidate.content.role == "model" {
            "assistant"
        } else {
            &candidate.content.role
        };
        
        let (content, reasoning_content) = extract_content_and_reasoning(&candidate.content.parts);
        let message = build_message_with_reasoning(role, content, reasoning_content);

        OpenAIChatCompletionChoice {
            index: candidate.index,
            message,
            finish_reason: candidate.finish_reason.clone(),
        }
    }).collect();

    OpenAIChatCompletionResponse {
        id,
        object: "chat.completion".to_string(),
        created,
        model: model.to_string(),
        choices,
        usage: None,
    }
}

pub fn gemini_stream_to_openai_stream(
    gemini_chunk: &GeminiStreamChunk,
    model: &str,
    response_id: &str
) -> OpenAIChatCompletionStreamResponse {
    let created = Utc::now().timestamp();

    let choices = gemini_chunk.candidates.iter().map(|candidate| {
        let (content_text, reasoning_text) = extract_content_and_reasoning(&candidate.content.parts);
        
        let content = if content_text.is_empty() { None } else { Some(content_text) };
        let reasoning_content = if reasoning_text.is_empty() { None } else { Some(reasoning_text) };
        
        OpenAIChatCompletionStreamChoice {
            index: candidate.index,
            delta: OpenAIDelta {
                role: if candidate.index == 0 { Some("assistant".to_string()) } else { None },
                content,
                reasoning_content,
            },
            finish_reason: candidate.finish_reason.clone(),
        }
    }).collect();

    OpenAIChatCompletionStreamResponse {
        id: response_id.to_string(),
        object: "chat.completion.chunk".to_string(),
        created,
        model: model.to_string(),
        choices,
    }
}