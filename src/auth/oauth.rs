// OAuth functionality is currently unused but kept for future features
use serde::{Deserialize, Serialize};
use anyhow::{Result, Context};

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
            client_id: "764086051850-6qr4p6gpi6hn506pt8ejuq83di341hur.apps.googleusercontent.com".to_string(),
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

        format!("https://accounts.google.com/o/oauth2/v2/auth?{}", query_string)
    }

    pub async fn exchange_code_for_tokens(&self, code: &str) -> Result<serde_json::Value> {
        let token_request = serde_json::json!({
            "client_id": self.config.client_id,
            "client_secret": self.config.client_secret,
            "code": code,
            "grant_type": "authorization_code",
            "redirect_uri": self.config.redirect_uri,
        });

        let response = self.http_client
            .post("https://oauth2.googleapis.com/token")
            .json(&token_request)
            .send()
            .await
            .context("Failed to exchange code for tokens")?;

        if response.status().is_success() {
            let tokens: serde_json::Value = response.json().await
                .context("Failed to parse token response")?;
            Ok(tokens)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            Err(anyhow::anyhow!("Token exchange failed: {}", error_text))
        }
    }

    pub async fn get_user_info(&self, access_token: &str) -> Result<serde_json::Value> {
        let response = self.http_client
            .get("https://www.googleapis.com/oauth2/v2/userinfo")
            .bearer_auth(access_token)
            .send()
            .await
            .context("Failed to get user info")?;

        if response.status().is_success() {
            let user_info: serde_json::Value = response.json().await
                .context("Failed to parse user info response")?;
            Ok(user_info)
        } else {
            let error_text = response.text().await.unwrap_or_default();
            Err(anyhow::anyhow!("Failed to get user info: {}", error_text))
        }
    }

    pub async fn get_project_id(&self, access_token: &str) -> Result<Option<String>> {
        let response = self.http_client
            .get("https://cloudresourcemanager.googleapis.com/v1/projects")
            .bearer_auth(access_token)
            .send()
            .await
            .context("Failed to get projects")?;

        if response.status().is_success() {
            let projects_response: serde_json::Value = response.json().await
                .context("Failed to parse projects response")?;
            
            if let Some(projects) = projects_response.get("projects").and_then(|p| p.as_array())
                && let Some(first_project) = projects.first()
                    && let Some(project_id) = first_project.get("projectId").and_then(|p| p.as_str()) {
                        return Ok(Some(project_id.to_string()));
                    }
        }
        
        Ok(None)
    }
}

 
pub fn get_auth_page_html(password: &str) -> String {
    format!(r#"
<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>GeminiCLI OAuth认证</title>
    <style>
        * {{
            box-sizing: border-box;
            margin: 0;
            padding: 0;
        }}
        
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            padding: 20px;
        }}
        
        .container {{
            background: white;
            border-radius: 12px;
            box-shadow: 0 20px 40px rgba(0,0,0,0.1);
            padding: 40px;
            max-width: 500px;
            width: 100%;
            text-align: center;
        }}
        
        .title {{
            color: #333;
            font-size: 28px;
            font-weight: 600;
            margin-bottom: 12px;
        }}
        
        .subtitle {{
            color: #666;
            font-size: 16px;
            margin-bottom: 30px;
            line-height: 1.5;
        }}
        
        .auth-form {{
            margin-bottom: 30px;
        }}
        
        .input-group {{
            margin-bottom: 20px;
            text-align: left;
        }}
        
        .input-label {{
            display: block;
            color: #555;
            font-weight: 500;
            margin-bottom: 8px;
        }}
        
        .input-field {{
            width: 100%;
            padding: 12px 16px;
            border: 2px solid #e1e5e9;
            border-radius: 8px;
            font-size: 16px;
            transition: border-color 0.2s ease;
        }}
        
        .input-field:focus {{
            outline: none;
            border-color: #667eea;
        }}
        
        .btn {{
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            border: none;
            padding: 14px 32px;
            border-radius: 8px;
            font-size: 16px;
            font-weight: 600;
            cursor: pointer;
            transition: transform 0.2s ease, box-shadow 0.2s ease;
            width: 100%;
        }}
        
        .btn:hover {{
            transform: translateY(-2px);
            box-shadow: 0 8px 16px rgba(102, 126, 234, 0.3);
        }}
        
        .btn:active {{
            transform: translateY(0);
        }}
        
        .info-section {{
            background: #f8f9fa;
            border-radius: 8px;
            padding: 20px;
            margin-top: 30px;
            text-align: left;
        }}
        
        .info-title {{
            color: #333;
            font-weight: 600;
            margin-bottom: 12px;
        }}
        
        .info-item {{
            color: #666;
            margin-bottom: 8px;
            font-size: 14px;
        }}
        
        .error {{
            background: #fee;
            color: #c33;
            padding: 12px;
            border-radius: 6px;
            margin-bottom: 20px;
            border: 1px solid #fcc;
        }}
        
        .success {{
            background: #efe;
            color: #363;
            padding: 12px;
            border-radius: 6px;
            margin-bottom: 20px;
            border: 1px solid #cfc;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1 class="title">GeminiCLI OAuth认证</h1>
        <p class="subtitle">请输入密码以访问OAuth认证页面</p>
        
        <div id="message"></div>
        
        <form class="auth-form" onsubmit="authenticate(event)">
            <div class="input-group">
                <label class="input-label" for="password">密码</label>
                <input type="password" id="password" class="input-field" 
                       placeholder="请输入密码" required>
            </div>
            <button type="submit" class="btn">验证并开始OAuth认证</button>
        </form>
        
        <div class="info-section">
            <div class="info-title">使用说明：</div>
            <div class="info-item">• 默认密码：{}</div>
            <div class="info-item">• 通过环境变量 PASSWORD 自定义密码</div>
            <div class="info-item">• 认证成功后即可使用 OpenAI 兼容 API</div>
            <div class="info-item">• API 地址：http://127.0.0.1:7878/v1</div>
        </div>
    </div>
    
    <script>
        function authenticate(event) {{
            event.preventDefault();
            const password = document.getElementById('password').value;
            const messageDiv = document.getElementById('message');
            
            if (password === '{}') {{
                messageDiv.innerHTML = '<div class="success">密码正确，正在跳转到OAuth认证...</div>';
                setTimeout(() => {{
                    window.location.href = '/auth/login';
                }}, 1000);
            }} else {{
                messageDiv.innerHTML = '<div class="error">密码错误，请重试</div>';
            }}
        }}
    </script>
</body>
</html>
"#, password, password)
}