use crate::models::GeminiThinkingConfig;

pub fn get_base_model_name(model_name: &str) -> String {
    let suffixes = ["-maxthinking", "-nothinking"];
    for suffix in &suffixes {
        if let Some(stripped) = model_name.strip_suffix(suffix) {
            return stripped.to_string();
        }
    }
    model_name.to_string()
}

pub fn is_nothinking_model(model_name: &str) -> bool {
    model_name.contains("-nothinking")
}

pub fn is_maxthinking_model(model_name: &str) -> bool {
    model_name.contains("-maxthinking")
}

pub fn get_thinking_budget(model_name: &str) -> Option<i32> {
    if is_nothinking_model(model_name) {
        Some(128)
    } else if is_maxthinking_model(model_name) {
        Some(32768)
    } else {
        Some(-1)
    }
}

pub fn should_include_thoughts(model_name: &str) -> bool {
    if is_nothinking_model(model_name) {
        let base_model = get_base_model_name(model_name);
        base_model.contains("gemini-2.5-pro")
    } else {
        true
    }
}

pub fn is_image_model(model_name: &str) -> bool {
    model_name.contains("gemini-2.5-flash-image")
}

pub fn get_thinking_config(model_name: &str) -> Option<GeminiThinkingConfig> {
    // Image models don't support thinking
    if is_image_model(model_name) {
        return None;
    }

    let thinking_budget = get_thinking_budget(model_name)?;
    let include_thoughts = should_include_thoughts(model_name);

    Some(GeminiThinkingConfig {
        thinking_budget,
        include_thoughts,
    })
}
