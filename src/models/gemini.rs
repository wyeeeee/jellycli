use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiInlineData {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub data: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiPartData {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thought: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "inlineData")]
    pub inline_data: Option<GeminiInlineData>,
}

impl Default for GeminiPartData {
    fn default() -> Self {
        Self {
            text: Some(String::new()),
            thought: Some(false),
            inline_data: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiPart {
    #[serde(flatten, default)]
    pub data: GeminiPartData,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiContent {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub parts: Vec<GeminiPart>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiRequest {
    pub contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generation_config: Option<GeminiGenerationConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_settings: Option<Vec<GeminiSafetySetting>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "thinkingConfig")]
    pub thinking_config: Option<GeminiThinkingConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiThinkingConfig {
    #[serde(rename = "thinkingBudget")]
    pub thinking_budget: i32,
    #[serde(rename = "includeThoughts")]
    pub include_thoughts: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiSafetySetting {
    pub category: String,
    pub threshold: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiCandidate {
    #[serde(default)]
    pub content: GeminiContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(default)]
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_ratings: Option<Vec<GeminiSafetyRating>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiSafetyRating {
    pub category: String,
    pub probability: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiResponse {
    #[serde(default)]
    pub candidates: Vec<GeminiCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_metadata: Option<GeminiUsageMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiUsageMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_token_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub candidates_token_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_token_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeminiStreamChunk {
    #[serde(default)]
    pub candidates: Vec<GeminiCandidate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_metadata: Option<GeminiUsageMetadata>,
}

impl GeminiPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            data: GeminiPartData {
                text: Some(text.into()),
                thought: Some(false),
                inline_data: None,
            },
        }
    }

    pub fn text_with_thought(text: impl Into<String>, thought: bool) -> Self {
        Self {
            data: GeminiPartData {
                text: Some(text.into()),
                thought: Some(thought),
                inline_data: None,
            },
        }
    }

    pub fn inline_data(mime_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            data: GeminiPartData {
                text: None,
                thought: None,
                inline_data: Some(GeminiInlineData {
                    mime_type: mime_type.into(),
                    data: data.into(),
                }),
            },
        }
    }

    pub fn get_text(&self) -> Option<&str> {
        match &self.data {
            GeminiPartData { text: Some(text), .. } => Some(text),
            _ => None,
        }
    }

    pub fn is_thought(&self) -> bool {
        match &self.data {
            GeminiPartData { thought: Some(thought), .. } => *thought,
            _ => false,
        }
    }

    pub fn get_inline_data(&self) -> Option<&GeminiInlineData> {
        match &self.data {
            GeminiPartData { inline_data: Some(inline_data), .. } => Some(inline_data),
            _ => None,
        }
    }

    pub fn is_empty_text(&self) -> bool {
        match &self.data {
            GeminiPartData { text: Some(text), .. } => text.is_empty(),
            GeminiPartData { text: None, .. } => true,
        }
    }
}
