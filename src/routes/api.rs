use axum::{
    Router,
    extract::State,
    middleware,
    response::{IntoResponse, Json},
    routing::{get, post},
};
use std::sync::Arc;

use crate::auth::auth_middleware;
use crate::client::GeminiCliService;
use crate::models::{Model, ModelList, OpenAIChatCompletionRequest};
use crate::utils::get_supported_models;

pub type AppState = Arc<GeminiCliService>;

pub fn create_api_routes() -> Router<AppState> {
    Router::new()
        .route("/v1/models", get(list_models))
        .route("/v1/chat/completions", post(chat_completions))
        .layer(middleware::from_fn(auth_middleware))
}

async fn list_models() -> Json<ModelList> {
    let models = get_supported_models()
        .into_iter()
        .map(|id| Model {
            id,
            object: "model".to_string(),
            created: chrono::Utc::now().timestamp(),
        })
        .collect();

    Json(ModelList {
        object: "list".to_string(),
        data: models,
    })
}

async fn chat_completions(
    State(service): State<AppState>,
    Json(request): Json<OpenAIChatCompletionRequest>,
) -> axum::response::Result<axum::response::Response> {
    match service.chat_completion(request).await {
        Ok(response) => Ok(response),
        Err((status, json)) => {
            let response = (status, json).into_response();
            Ok(response)
        }
    }
}
