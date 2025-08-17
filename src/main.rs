mod client;
mod models;
mod routes;
mod utils;
mod auth;

use axum::Router;
use std::sync::Arc;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;
use tracing::{info, error};

use crate::auth::{CredentialManager, init_auth_config};
use crate::client::GeminiCliService;
use crate::routes::create_api_routes;
use crate::utils::{AppConfig, init_logger};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    init_logger();

    // Load configuration
    let config = Arc::new(AppConfig::from_file());
    
    // Initialize auth config for middleware
    init_auth_config(Arc::clone(&config));
    
    info!("Starting gcli2api server...");
    info!("Configuration:");
    info!("  Bind address: {}", config.bind_address);
    info!("  Credentials directory: {}", config.credentials_dir);
    info!("  Calls per rotation: {}", config.calls_per_rotation);
    info!("  Max retries: {}", config.max_retries);

    // Initialize credential manager
    let credential_manager = CredentialManager::new(
        &config.credentials_dir,
        config.calls_per_rotation,
        config.max_retries,
    );

    // Initialize GeminiCLI service
    let service = GeminiCliService::new(
        credential_manager,
        config.code_assist_endpoint.clone(),
    );

    if let Err(e) = service.initialize().await {
        error!("Failed to initialize GeminiCLI service: {}", e);
    }

    let app_state = Arc::new(service);

    // Create routers
    let api_router = create_api_routes();
    let health_router = routes::create_health_routes();

    // Combine all routers  
    let app = Router::new()
        .merge(health_router)
        .merge(api_router.with_state(Arc::clone(&app_state)))
        .layer(
            ServiceBuilder::new()
                .layer(CorsLayer::permissive())
        );

    // Start server
    let listener = tokio::net::TcpListener::bind(&config.bind_address)
        .await
        .map_err(|e| {
            error!("Failed to bind to address {}: {}", config.bind_address, e);
            e
        })?;

    info!("🚀 Server started successfully!");
    info!("📍 API endpoint: http://{}/v1", config.bind_address);
    info!("💊 Health check: http://{}/health", config.bind_address);
    info!("🔑 Default password: {}", config.password);

    axum::serve(listener, app)
        .await
        .map_err(|e| {
            error!("Server error: {}", e);
            e
        })?;

    Ok(())
}