use axum::{
    extract::Request,
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::Response,
};
use axum::response::Json;
use std::sync::{Arc, OnceLock};
use crate::models::ErrorResponse;
use crate::utils::AppConfig;

static CONFIG: OnceLock<Arc<AppConfig>> = OnceLock::new();

pub fn init_auth_config(config: Arc<AppConfig>) {
    CONFIG.set(config).ok();
}

pub async fn auth_middleware(
    request: Request,
    next: Next,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let headers = request.headers();
    let config = CONFIG.get().expect("Config not initialized");
    let password = &config.password;
    
    if let Some(auth_header) = headers.get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                let token = &auth_str[7..];
                if token == password {
                    return Ok(next.run(request).await);
                }
            }
        }
    }
    
    Err((
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: crate::models::ApiError {
                message: "密码错误".to_string(),
                error_type: "authentication_error".to_string(),
                code: 403,
            },
        }),
    ))
}

 
pub fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get("authorization")?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

 
pub fn validate_password(token: &str, config_password: &str) -> bool {
    token == config_password
}