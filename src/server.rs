pub mod handlers;
pub mod models;
pub mod services;
mod utils;

use self::handlers::{sse_handler, websocket_handler};
use self::models::AppState;
use self::services::{Ntfy, cleaner, rate_fetcher};
use self::utils::{load_ntfy_config, wait_for_shutdown_signal};

use axum::routing::get;
use dioxus::logger::tracing::{info, warn};
use dioxus::prelude::*;
use dioxus::server::ServeConfigBuilder;

async fn init_db() -> sqlx::SqlitePool {
    // Initialize database
    let db = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:rates.db".to_string());
    info!("Database URL: {db}");
    let db_pool = sqlx::SqlitePool::connect(&db).await.unwrap();

    // Run migrations
    sqlx::migrate!("./migrations").run(&db_pool).await.unwrap();
    db_pool
}

fn init_ntfy() -> Option<Ntfy> {
    let ntfy_config = load_ntfy_config();
    if let Some(config) = ntfy_config {
        // Initialize Ntfy client with the provided configuration
        let mut client_builder =
            reqwest::Client::builder().timeout(std::time::Duration::from_secs(10));
        if let Some(auth) = &config.auth {
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(auth).unwrap(),
            );
            client_builder = client_builder.default_headers(headers);
        }
        let client = client_builder.build().unwrap();
        Some(Ntfy::new(client, config))
    } else {
        warn!("No Ntfy config found.");
        None
    }
}

pub async fn launch_server(component: fn() -> Element) {
    dotenv::dotenv().ok();

    let db_pool = init_db().await;

    let state = AppState::new(db_pool);

    // Load existing data from database
    if let Err(e) = state.load_from_db().await {
        eprintln!("Failed to load history from database: {e}");
    }

    let ntfy = init_ntfy();

    let mut tasks = Vec::new();

    tasks.push(tokio::spawn(cleaner(state.clone())));

    // Start the rate fetcher in the background
    tasks.push(tokio::spawn(rate_fetcher(state.clone(), ntfy)));

    tasks.push(tokio::spawn(async move {
        // Get the address the server should run on. If the CLI is running, the CLI proxies fullstack into the main address
        // and we use the generated address the CLI gives us
        let addr = dioxus::cli_config::fullstack_address_or_localhost();
        info!("Launching server on {addr}");
        let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

        // Add server state to dioxus context, which can be accessed by server functions
        let conf = ServeConfigBuilder::new().context(state.clone());

        let router = axum::Router::new()
            // serve_dioxus_application adds routes to server side render the application, serve static assets, and register server functions
            .serve_dioxus_application(conf.build().unwrap(), component)
            .route("/ws", get(websocket_handler))
            .route("/sse", get(sse_handler))
            .with_state(state);
        axum::serve(listener, router).await.unwrap();
    }));

    // Gracefully wait for shutdown signal
    wait_for_shutdown_signal().await;
    for task in &tasks {
        task.abort();
    }
    for task in tasks {
        assert!(task.await.unwrap_err().is_cancelled());
    }
}
