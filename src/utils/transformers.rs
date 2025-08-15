use crate::models::{
    OpenAIChatCompletionRequest, OpenAIChatCompletionResponse, OpenAIChatCompletionChoice,
    OpenAIChatCompletionStreamResponse, OpenAIChatCompletionStreamChoice,
    OpenAIChatMessage, OpenAIDelta,
    GeminiRequest, GeminiContent, GeminiPart, GeminiGenerationConfig,
    GeminiResponse, GeminiStreamChunk
};
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
            parts: vec![GeminiPart { text }],
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
        })
    } else {
        None
    };

    GeminiRequest {
        contents,
        generation_config,
        safety_settings: None,
    }
}

pub fn gemini_to_openai_response(
    gemini_resp: &GeminiResponse, 
    model: &str
) -> OpenAIChatCompletionResponse {
    let id = format!("chatcmpl-{}", Uuid::new_v4());
    let created = Utc::now().timestamp();

    let choices = gemini_resp.candidates.iter().map(|candidate| {
        let content = candidate.content.parts.iter()
            .map(|part| part.text.as_str())
            .collect::<Vec<_>>()
            .join("");

        OpenAIChatCompletionChoice {
            index: candidate.index,
            message: OpenAIChatMessage {
                role: "assistant".to_string(),
                content: Value::String(content),
                reasoning_content: None,
            },
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
        let content = if candidate.content.parts.is_empty() {
            None
        } else {
            Some(candidate.content.parts.iter()
                .map(|part| part.text.as_str())
                .collect::<Vec<_>>()
                .join(""))
        };

        OpenAIChatCompletionStreamChoice {
            index: candidate.index,
            delta: OpenAIDelta {
                role: if candidate.index == 0 { Some("assistant".to_string()) } else { None },
                content,
                reasoning_content: None,
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