use axum::{Router, routing::get};
use dotenv::dotenv;
use sqlx::sqlite::SqlitePool;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;

#[cfg(not(unix))]
use tokio::signal;
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};

use rated::{
    AppState, Ntfy, api_history, api_latest, cleaner, home_page, load_ntfy_config, rate_fetcher,
    websocket_handler,
};

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm = signal(SignalKind::terminate()).unwrap();
        let mut sigint = signal(SignalKind::interrupt()).unwrap();

        tokio::select! {
            _ = sigterm.recv() => {
                println!("\nReceived SIGTERM (systemd stop/restart)");
            }
            _ = sigint.recv() => {
                println!("\nReceived SIGINT (Ctrl+C)");
            }
        }
    }

    #[cfg(not(unix))]
    {
        // fallback for non-UNIX systems
        signal::ctrl_c().await.unwrap();
        println!("Received Ctrl+C");
    }
}

#[tokio::main]
async fn main() {
    // Load environment variables from .env file if present
    dotenv().ok();

    // Initialize database
    let db = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:rates.db".to_string());
    println!("Database URL: {db}");
    let db_pool = SqlitePool::connect(&db).await.unwrap();

    // Run migrations
    sqlx::migrate!("./migrations").run(&db_pool).await.unwrap();

    let ntfy_config = load_ntfy_config();

    let ntfy = if let Some(config) = ntfy_config {
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
        None
    };

    let state = AppState::new(db_pool);

    // Load existing data from database
    if let Err(e) = state.load_from_db().await {
        eprintln!("Failed to load history from database: {e}");
    }

    let mut tasks = Vec::new();

    tasks.push(tokio::spawn(cleaner(state.clone())));

    // Start the rate fetcher in the background
    tasks.push(tokio::spawn(rate_fetcher(state.clone(), ntfy)));

    let server_state = state.clone();
    tasks.push(tokio::spawn(async move {
        // Build the router
        let app = Router::new()
            .route("/", get(home_page))
            .route("/ws", get(websocket_handler))
            .route("/api/latest", get(api_latest))
            .route("/api/history", get(api_history))
            .layer(ServiceBuilder::new().layer(CorsLayer::permissive()))
            .with_state(server_state);

        // Start the server
        let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{port}"))
            .await
            .unwrap();

        println!("Server is running on http://127.0.0.1:{port}");

        axum::serve(listener, app).await.unwrap();
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
