use crate::models::{
    GeminiContent, GeminiGenerationConfig, GeminiPart, GeminiRequest, GeminiResponse,
    GeminiStreamChunk, OpenAIChatCompletionChoice, OpenAIChatCompletionRequest,
    OpenAIChatCompletionResponse, OpenAIChatCompletionStreamChoice,
    OpenAIChatCompletionStreamResponse, OpenAIChatMessage, OpenAIDelta,
};
use crate::utils::thinking_config::*;
use chrono::Utc;
use serde_json::Value;
use uuid::Uuid;

fn parse_content_parts(content: &Value) -> Vec<GeminiPart> {
    match content {
        Value::String(s) => {
            // Parse markdown images in text content
            parse_text_with_images(s)
        }
        Value::Array(arr) => {
            // Handle OpenAI's array format for multimodal content
            let mut parts = Vec::new();

            for item in arr {
                if let Value::Object(obj) = item {
                    // Handle text parts
                    if let Some(Value::String(text)) = obj.get("text") {
                        if !text.trim().is_empty() {
                            parts.append(&mut parse_text_with_images(text));
                        }
                    }

                    // Handle image_url parts (OpenAI format)
                    if let Some(image_url_obj) = obj.get("image_url") {
                        if let Some(Value::String(image_url)) = image_url_obj.get("url") {
                            if let Some(part) = parse_image_url(image_url) {
                                parts.push(part);
                            }
                        }
                    }
                }
            }

            parts
        }
        _ => vec![GeminiPart::text(content.to_string())],
    }
}

fn parse_text_with_images(text: &str) -> Vec<GeminiPart> {
    let mut parts = Vec::new();

    // Regular expression to match markdown images: ![alt](url)
    let re = regex::Regex::new(r"!\[([^\]]*)\]\(([^)]+)\)").unwrap();

    let mut last_end = 0;

    for cap in re.captures_iter(text) {
        let full_match = cap.get(0).unwrap();
        let _alt_text = cap.get(1).unwrap().as_str();
        let url = cap.get(2).unwrap().as_str();

        // Add text before the image
        if full_match.start() > last_end {
            let text_before = &text[last_end..full_match.start()];
            if !text_before.trim().is_empty() {
                parts.push(GeminiPart::text(text_before));
            }
        }

        // Handle the image
        if let Some(image_part) = parse_image_url(url) {
            parts.push(image_part);
        } else {
            // If image parsing fails, keep the original markdown
            parts.push(GeminiPart::text(full_match.as_str().to_string()));
        }

        last_end = full_match.end();
    }

    // Add remaining text
    if last_end < text.len() {
        let remaining_text = &text[last_end..];
        if !remaining_text.trim().is_empty() {
            parts.push(GeminiPart::text(remaining_text));
        }
    }

    // If no images were found, just return the text as a single part
    if parts.is_empty() {
        parts.push(GeminiPart::text(text));
    }

    parts
}

fn parse_image_url(url: &str) -> Option<GeminiPart> {
    // Handle data URI images: data:image/png;base64,xxxxx
    if url.starts_with("data:") {
        if let Ok((mime_type, data)) = parse_data_uri(url) {
            return Some(GeminiPart::inline_data(mime_type, data));
        }
    }

    // For non-data URIs, we can't process them without fetching
    // So we return None and let the caller handle it
    None
}

fn parse_data_uri(url: &str) -> Result<(String, String), &'static str> {
    // Expected format: data:image/png;base64,xxxxx
    if !url.starts_with("data:") {
        return Err("Invalid data URI format");
    }

    // Remove "data:" prefix
    let rest = &url[5..];

    // Split at comma to separate metadata from data
    let parts: Vec<&str> = rest.splitn(2, ',').collect();
    if parts.len() != 2 {
        return Err("Invalid data URI format - missing comma");
    }

    let metadata = parts[0];
    let data = parts[1];

    // Parse mime type from metadata (e.g., "image/png;base64" -> "image/png")
    let mime_parts: Vec<&str> = metadata.split(';').collect();
    let mime_type = mime_parts.first().unwrap_or(&"image/png");

    // Default to image/png if mime type is empty
    let final_mime_type = if mime_type.is_empty() { "image/png" } else { mime_type };

    Ok((final_mime_type.to_string(), data.to_string()))
}


pub fn openai_to_gemini_request(openai_req: &OpenAIChatCompletionRequest) -> GeminiRequest {
    let contents = openai_req
        .messages
        .iter()
        .map(|msg| {
            let role = match msg.role.as_str() {
                "system" => "user".to_string(),
                "assistant" => "model".to_string(),
                _ => msg.role.clone(),
            };

            let parts = parse_content_parts(&msg.content);

            GeminiContent { role, parts }
        })
        .collect();

    let generation_config = if openai_req.temperature.is_some()
        || openai_req.top_p.is_some()
        || openai_req.max_tokens.is_some()
        || openai_req.stop.is_some()
    {
        Some(GeminiGenerationConfig {
            temperature: openai_req.temperature,
            top_p: openai_req.top_p,
            top_k: None,
            max_output_tokens: openai_req.max_tokens,
            stop_sequences: openai_req.stop.as_ref().and_then(|stop| match stop {
                Value::String(s) => Some(vec![s.clone()]),
                Value::Array(arr) => Some(
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect(),
                ),
                _ => None,
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
    let mut content_parts = Vec::new();
    let mut reasoning_content = String::new();

    // Debug: Print the parts we received
    println!("🔍 DEBUG: extract_content_and_reasoning received {} parts", parts.len());
    for (i, part) in parts.iter().enumerate() {
        match &part.data {
            crate::models::GeminiPartData { text: Some(text), thought: Some(thought), inline_data: None } => {
                println!("🔍 DEBUG: Part {} - Text: '{}' ({} chars, thought: {})", i, text, text.len(), thought);
                if !text.is_empty() {
                    if *thought {
                        reasoning_content.push_str(text);
                    } else {
                        content_parts.push(text.clone());
                    }
                }
            }
            crate::models::GeminiPartData { text: Some(text), thought: None, inline_data: None } => {
                println!("🔍 DEBUG: Part {} - Text: '{}' ({} chars, thought: None)", i, text, text.len());
                if !text.is_empty() {
                    content_parts.push(text.clone());
                }
            }
            crate::models::GeminiPartData { text: None, thought: None, inline_data: Some(inline_data) } => {
                println!("🔍 DEBUG: Part {} - Image: {} ({} bytes)", i, inline_data.mime_type, inline_data.data.len());
                // Convert inline image data back to markdown format
                if !inline_data.data.is_empty() {
                    let markdown_image = format!(
                        "![image](data:{};base64,{})",
                        inline_data.mime_type, inline_data.data
                    );
                    println!("🔍 DEBUG: Adding markdown image: {}", &markdown_image[..50]);
                    content_parts.push(markdown_image);
                }
            }
            crate::models::GeminiPartData { text: None, thought: None, inline_data: None } => {
                // Empty part, skip
                println!("🔍 DEBUG: Part {} - Empty part", i);
            }
            _ => {
                // Handle edge cases
                println!("🔍 DEBUG: Part {} - Unexpected data structure: {:?}", i, part.data);
            }
        }
    }

    let content = content_parts.join("\n\n");
    println!("🔍 DEBUG: Final content length: {} chars", content.len());
    if content.len() > 100 {
        println!("🔍 DEBUG: Final content preview: {}", &content[..100]);
    } else {
        println!("🔍 DEBUG: Final content: {}", content);
    }

    (content, reasoning_content)
}

fn build_message_with_reasoning(
    role: &str,
    content: String,
    reasoning_content: String,
) -> OpenAIChatMessage {
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
    model: &str,
) -> OpenAIChatCompletionResponse {
    let id = format!("chatcmpl-{}", Uuid::new_v4());
    let created = Utc::now().timestamp();

    let choices = gemini_resp
        .candidates
        .iter()
        .map(|candidate| {
            let role = if candidate.content.role == "model" {
                "assistant"
            } else {
                &candidate.content.role
            };

            let (content, reasoning_content) =
                extract_content_and_reasoning(&candidate.content.parts);
            let message = build_message_with_reasoning(role, content, reasoning_content);

            OpenAIChatCompletionChoice {
                index: candidate.index,
                message,
                finish_reason: candidate.finish_reason.clone(),
            }
        })
        .collect();

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
    response_id: &str,
) -> OpenAIChatCompletionStreamResponse {
    let created = Utc::now().timestamp();

    let choices = gemini_chunk
        .candidates
        .iter()
        .map(|candidate| {
            let (content_text, reasoning_text) =
                extract_content_and_reasoning(&candidate.content.parts);

            let content = if content_text.is_empty() {
                None
            } else {
                Some(content_text)
            };
            let reasoning_content = if reasoning_text.is_empty() {
                None
            } else {
                Some(reasoning_text)
            };

            OpenAIChatCompletionStreamChoice {
                index: candidate.index,
                delta: OpenAIDelta {
                    role: if candidate.index == 0 {
                        Some("assistant".to_string())
                    } else {
                        None
                    },
                    content,
                    reasoning_content,
                },
                finish_reason: candidate.finish_reason.clone(),
            }
        })
        .collect();

    OpenAIChatCompletionStreamResponse {
        id: response_id.to_string(),
        object: "chat.completion.chunk".to_string(),
        created,
        model: model.to_string(),
        choices,
    }
}
