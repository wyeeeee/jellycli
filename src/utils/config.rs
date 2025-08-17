use serde::{Deserialize, Serialize};
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub password: String,
    pub bind_address: String,
    pub credentials_dir: String,
    pub code_assist_endpoint: String,
    pub calls_per_rotation: usize,
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
}

fn default_max_retries() -> usize {
    3
}

impl AppConfig {
    pub fn from_file() -> Self {
        // Try to read config.json, fallback to default if not found
        if let Ok(content) = fs::read_to_string("config.json") {
            if let Ok(config) = serde_json::from_str(&content) {
                return config;
            }
        }
        
        // Default configuration
        Self {
            password: "pwd".to_string(),
            bind_address: "0.0.0.0:7878".to_string(),
            credentials_dir: "./credentials".to_string(),
            code_assist_endpoint: "https://codeassist-pa.clients6.google.com".to_string(),
            calls_per_rotation: 1,
            max_retries: default_max_retries(),
        }
    }
}

pub fn get_supported_models() -> Vec<String> {
    vec![
        "gemini-2.5-pro-preview-06-05".to_string(),
        "gemini-2.5-pro-preview-06-05-假流式".to_string(),
        "gemini-2.5-pro".to_string(),
        "gemini-2.5-pro-假流式".to_string(),
        "gemini-2.5-pro-preview-05-06".to_string(),
        "gemini-2.5-pro-preview-05-06-假流式".to_string(),
    ]
}

pub fn get_user_agent() -> String {
    let version = "0.1.5";  // Match Python version
    let system = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    format!("GeminiCLI/{} ({system}; {arch})", version)
}

pub fn get_client_metadata(project_id: &str) -> serde_json::Value {
    serde_json::json!({
        "ideType": "IDE_UNSPECIFIED",
        "platform": get_platform_string(),
        "pluginType": "GEMINI",
        "duetProject": project_id,
    })
}

pub fn get_platform_string() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    
    match (os, arch) {
        ("macos", "aarch64") => "DARWIN_ARM64".to_string(),
        ("macos", _) => "DARWIN_AMD64".to_string(),
        ("linux", "aarch64") => "LINUX_ARM64".to_string(),
        ("linux", _) => "LINUX_AMD64".to_string(),
        ("windows", _) => "WINDOWS_AMD64".to_string(),
        _ => "PLATFORM_UNSPECIFIED".to_string(),
    }
}