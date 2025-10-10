// OAuth functionality is currently unused but kept for future features
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]

pub struct OAuthCallback {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
    pub scope: String,
}

impl Default for OAuthConfig {
    fn default() -> Self {
        Self {
            client_id: "764086051850-6qr4p6gpi6hn506pt8ejuq83di341hur.apps.googleusercontent.com"
                .to_string(),
            client_secret: "d-FL95Q19q7MQmFpd7hHD0Ty".to_string(),
            redirect_uri: "http://127.0.0.1:7878/auth/callback".to_string(),
            scope: "https://www.googleapis.com/auth/cloud-platform".to_string(),
        }
    }
}

pub struct OAuthService {
    config: OAuthConfig,
    http_client: reqwest::Client,
}

impl OAuthService {
    pub fn new() -> Self {
        Self {
            config: OAuthConfig::default(),
            http_client: reqwest::Client::new(),
        }
    }

    pub fn get_authorization_url(&self) -> String {
        let params = [
            ("client_id", &self.config.client_id),
            ("redirect_uri", &self.config.redirect_uri),
            ("scope", &self.config.scope),
            ("response_type", &"code".to_string()),
            ("access_type", &"offline".to_string()),
            ("prompt", &"consent".to_string()),
        ];

        let query_string = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        format!(
            "https://accounts.google.com/o/oauth2/v2/auth?{}",
            query_string
        )
    }

    pub async fn exchange_code_for_tokens(&self, code: &str) -> Result<serde_json::Value> {
        let token_request = serde_json::json!({
            "client_id": self.config.client_id,
            "client_secret": self.config.client_secret,
            "code": code,
            "grant_type": "authorization_code",
            "redirect_uri": self.config.redirect_uri,
        });

        let response = self
            .http_client
            .post("https://oauth2.googleapis.com/token")
            .json(&token_request)
            .send()
            .await
            .context("Failed to exchange code for tokens")?;

        if response.status().is_success() {
            let tokens: serde_json::Value = response
                .json()
                .await
                .context("Failed to parse token response")?;
            Ok(tokens)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            Err(anyhow::anyhow!("Token exchange failed: {}", error_text))
        }
    }

    pub async fn get_user_info(&self, access_token: &str) -> Result<serde_json::Value> {
        let response = self
            .http_client
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
            .bearer_auth(access_token)
            .send()
            .await
            .context("Failed to get user info")?;

        if response.status().is_success() {
            let user_info: serde_json::Value = response
                .json()
                .await
                .context("Failed to parse user info response")?;
            Ok(user_info)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            Err(anyhow::anyhow!("Failed to get user info: {}", error_text))
        }
    }

    pub async fn get_project_id(&self, access_token: &str) -> Result<Option<String>> {
        let response = self
            .http_client
            .get("https://cloudresourcemanager.googleapis.com/v1/projects")
            .bearer_auth(access_token)
            .send()
            .await
            .context("Failed to get projects")?;

        if response.status().is_success() {
            let projects_response: serde_json::Value = response
                .json()
                .await
                .context("Failed to parse projects response")?;

            if let Some(projects) = projects_response.get("projects").and_then(|p| p.as_array())
                && let Some(first_project) = projects.first()
                && let Some(project_id) = first_project.get("projectId").and_then(|p| p.as_str())
            {
                return Ok(Some(project_id.to_string()));
            }
        }

        Ok(None)
    }
}
