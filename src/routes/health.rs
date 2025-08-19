use axum::{Router, response::Json, routing::get};
use serde_json::{Value, json};

pub fn create_health_routes() -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/", get(root_handler))
}

async fn health_check() -> Json<Value> {
    Json(json!({
        "status": "healthy",
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "service": "gcli2api",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn root_handler() -> Json<Value> {
    Json(json!({
        "message": "GeminiCLI to OpenAI API Bridge",
        "version": env!("CARGO_PKG_VERSION"),
        "endpoints": {
            "api": "http://127.0.0.1:7878/v1",
            "health": "http://127.0.0.1:7878/health",
        },
        "documentation": "https://github.com/wyeeeee/jellycli"
    }))
}
