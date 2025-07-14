use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;
use std::{collections::VecDeque, io::Write};

use askama::Template;
use axum::{
    Router,
    extract::{State, WebSocketUpgrade, ws::WebSocket},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
};
use chrono::{DateTime, Utc};
use futures_util::{sink::SinkExt, stream::StreamExt};
use regex::Regex;
use reqwest::{
    Client,
    header::{ETAG, HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED},
};
use serde::{Deserialize, Serialize};
use sqlx::{QueryBuilder, Row, sqlite::SqlitePool};
use tokio::sync::broadcast;
use tokio::time::sleep;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;

#[cfg(not(unix))]
use tokio::signal;
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};

static URL: &'static str = "https://www.boc.cn/sourcedb/whpj/mfx_1620.html";

static RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<td class="mc">\s*澳大利亚元\s*</td>\s*<td[^>]*>(.*?)</td>\s*<td[^>]*>(.*?)</td>"#,
    )
    .unwrap()
});

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RateRecord {
    rate: String,
    timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug)]
struct RateDisplay {
    rate: String,
    timestamp: DateTime<Utc>,
    change_text: String,
    change_indicator: String,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    history: Vec<RateDisplay>,
}

#[derive(Clone, Debug)]
struct CacheInfo {
    etag: String,
    last_modified: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum WebSocketMessage {
    #[serde(rename = "rate_update")]
    RateUpdate { record: RateRecord },
    #[serde(rename = "history")]
    History { records: Vec<RateRecord> },
    #[serde(rename = "ping")]
    Ping,
    #[serde(rename = "pong")]
    Pong,
}

#[derive(Clone)]
struct AppState {
    rate_history: Arc<RwLock<VecDeque<RateRecord>>>,
    broadcast_tx: broadcast::Sender<WebSocketMessage>,
    db_pool: SqlitePool,
}

impl AppState {
    fn new(db_pool: SqlitePool) -> Self {
        let (broadcast_tx, _) = broadcast::channel(1000);

        Self {
            rate_history: Arc::new(RwLock::new(VecDeque::new())),
            broadcast_tx,
            db_pool,
        }
    }

    fn add_rate(&self, record: RateRecord) -> bool {
        {
            let history = self.rate_history.read().unwrap();

            if let Some(last_record) = history.back() {
                if last_record.timestamp == record.timestamp {
                    let mut stdout = std::io::stdout();
                    stdout.write(b"+").unwrap();
                    stdout.flush().unwrap();
                    return false;
                }
            }
        }

        println!(
            "+\n[{}] 澳元汇卖价: {}",
            record.timestamp.format("%Y-%m-%d %H:%M:%S"),
            record.rate
        );

        let should_persist = {
            let mut history = self.rate_history.write().unwrap();
            history.push_back(record.clone());

            // Check if we need to persist (when reaching 150 records)
            history.len() >= 150
        };

        // Broadcast the update to all connected WebSocket clients
        let message = WebSocketMessage::RateUpdate { record };
        let _ = self.broadcast_tx.send(message);

        should_persist
    }

    fn get_latest_rate(&self) -> Option<RateRecord> {
        let history = self.rate_history.read().unwrap();
        history.back().cloned()
    }

    fn get_history(&self) -> Vec<RateRecord> {
        let history = self.rate_history.read().unwrap();
        history.iter().rev().cloned().collect()
    }

    fn subscribe(&self) -> broadcast::Receiver<WebSocketMessage> {
        self.broadcast_tx.subscribe()
    }

    async fn persist_to_db(&self) -> Result<(), sqlx::Error> {
        let records_to_persist = {
            let history = self.rate_history.read().unwrap();
            history.iter().cloned().collect::<Vec<_>>()
        };

        if records_to_persist.is_empty() {
            return Ok(());
        }

        // Use QueryBuilder to batch insert
        let mut query_builder: QueryBuilder<sqlx::Sqlite> =
            QueryBuilder::new("INSERT OR IGNORE INTO rate_records (rate, timestamp) ");

        query_builder.push_values(&records_to_persist, |mut b, record| {
            b.push_bind(&record.rate).push_bind(record.timestamp);
        });

        let result = query_builder.build().execute(&self.db_pool).await?;
        println!("*\n已持久化 {} 条记录到数据库", result.rows_affected());

        // Keep only the latest 75 records in memory
        {
            let mut history = self.rate_history.write().unwrap();
            while history.len() > 75 {
                history.pop_front();
            }
        }

        Ok(())
    }

    async fn load_from_db(&self) -> Result<(), sqlx::Error> {
        let rows = sqlx::query(
            "SELECT rate, timestamp FROM rate_records ORDER BY timestamp DESC LIMIT 75",
        )
        .fetch_all(&self.db_pool)
        .await?;

        let mut history = self.rate_history.write().unwrap();
        history.clear();

        for row in rows.into_iter().rev() {
            let rate: String = row.get("rate");
            let timestamp: DateTime<Utc> = row.get("timestamp");

            history.push_back(RateRecord { rate, timestamp });
        }

        println!("📚 从数据库加载了 {} 条历史记录", history.len());
        Ok(())
    }
}

async fn fetch_sell_rate(
    client: &Client,
    cache_info: &Arc<RwLock<Option<CacheInfo>>>,
) -> Option<RateRecord> {
    let mut headers = HeaderMap::new();
    // 添加缓存头部
    {
        let cache = cache_info.read().unwrap();
        if let Some(cache) = &*cache {
            // println!("使用缓存：{:?}", &cache);
            if let Ok(header_value) = HeaderValue::from_str(&cache.etag) {
                headers.insert(IF_NONE_MATCH, header_value);
            } else {
                eprintln!("*\n警告: ETag 格式无效: {}", &cache.etag);
            }
            if let Ok(header_value) = HeaderValue::from_str(&cache.last_modified) {
                headers.insert(IF_MODIFIED_SINCE, header_value);
            } else {
                eprintln!("*\n警告: Last-Modified 格式无效: {}", &cache.last_modified);
            }
        }
    }

    let response = match client.get(URL).headers(headers).send().await {
        Ok(resp) => resp,
        Err(e) => {
            eprintln!("*\nHTTP 请求失败: {:?}", e);
            return None;
        }
    };

    let status = response.status().as_u16();
    // println!("HTTP 响应状态: {}", status);

    // 检查是否返回304 Not Modified，跳过
    if status == 304 {
        let mut stdout = std::io::stdout();
        stdout.write(b".").unwrap();
        stdout.flush().unwrap();
        return None;
    }

    // 更新缓存信息
    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let last_modified = response
        .headers()
        .get(LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let html = match response.text().await {
        Ok(content) => content,
        Err(e) => {
            eprintln!("*\n读取响应内容失败: {}", e);
            return None;
        }
    };

    // 匹配 "澳大利亚元" 行后的两个 <td>，第2个是卖出价
    if let Some(caps) = &RE.captures(&html) {
        let rate = caps.get(2)?.as_str().trim().to_string();
        // println!("成功提取汇率: {}", rate);
        // 更新缓存
        match (etag, &last_modified) {
            (Some(etag), Some(last_modified)) => {
                let mut cache = cache_info.write().unwrap();
                let cache_info = CacheInfo {
                    etag: etag,
                    last_modified: last_modified.clone(),
                };
                // println!("{:?}", cache_info);
                *cache = Some(cache_info);
            }
            _ => {
                eprintln!("*\n错误: 缺少必要的缓存信息");
            }
        }
        return match last_modified {
            Some(last_modified) => match DateTime::parse_from_rfc2822(&last_modified) {
                Ok(timestamp) => Some(RateRecord {
                    rate: rate,
                    timestamp: timestamp.to_utc(),
                }),
                Err(_) => {
                    eprintln!("*\n错误: 无法解析最后修改时间，使用当前时间");
                    Some(RateRecord {
                        rate: rate,
                        timestamp: Utc::now(),
                    })
                }
            },
            None => {
                eprintln!("*\n错误: 缺少最后修改时间信息，使用当前时间");
                Some(RateRecord {
                    rate: rate,
                    timestamp: Utc::now(),
                })
            }
        };
    } else {
        eprintln!("*\n错误: 在页面内容中找不到澳大利亚元汇率信息");
        return None;
    }
}

async fn rate_fetcher(state: AppState) {
    let fetch_interval = Duration::from_secs(1);
    println!(
        "🚀 汇率获取器已启动，每{}秒获取一次数据",
        fetch_interval.as_secs()
    );

    let client = Client::builder()
        .user_agent(
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.15; rv:141.0) Gecko/20100101 Firefox/141.0",
        )
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let cache_info: Arc<RwLock<Option<CacheInfo>>> = Arc::new(RwLock::new(None));

    loop {
        match fetch_sell_rate(&client, &cache_info).await {
            Some(current_rate) => {
                let should_persist = state.add_rate(current_rate);
                if should_persist {
                    if let Err(e) = state.persist_to_db().await {
                        eprintln!("❌ 持久化失败: {}", e);
                    }
                }
            }
            None => (),
        }
        sleep(fetch_interval).await;
    }
}

async fn periodic_persister(state: AppState) {
    let persist_interval = Duration::from_secs(300); // 5 minutes
    println!("⏰ 定时持久化器已启动，每5分钟执行一次");

    loop {
        sleep(persist_interval).await;
        let history_len = {
            let history = state.rate_history.read().unwrap();
            history.len()
        };

        if history_len > 0 {
            if let Err(e) = state.persist_to_db().await {
                eprintln!("❌ 定时持久化失败: {}", e);
            }
        }
    }
}

async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.subscribe();

    // Send initial history to the client
    let history = state.get_history();
    let history_message = WebSocketMessage::History { records: history };
    if let Ok(msg) = serde_json::to_string(&history_message) {
        if sender
            .send(axum::extract::ws::Message::Text(msg.into()))
            .await
            .is_err()
        {
            return;
        }
    }

    // Create a channel to send messages from receiver task to sender task
    let (tx, mut rx_internal) = tokio::sync::mpsc::unbounded_channel::<WebSocketMessage>();

    // Handle incoming messages from client
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            match msg {
                Ok(axum::extract::ws::Message::Text(text)) => {
                    // Handle ping/pong or other client messages
                    if let Ok(parsed) = serde_json::from_str::<WebSocketMessage>(&text) {
                        match parsed {
                            WebSocketMessage::Ping => {
                                let pong = WebSocketMessage::Pong;
                                if tx.send(pong).is_err() {
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ok(axum::extract::ws::Message::Close(_)) => {
                    break;
                }
                _ => {}
            }
        }
    });

    // Handle broadcast messages and internal messages to client
    let send_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                // Handle broadcast messages
                msg = rx.recv() => {
                    match msg {
                        Ok(msg) => {
                            if let Ok(json_msg) = serde_json::to_string(&msg) {
                                if sender.send(axum::extract::ws::Message::Text(json_msg.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                // Handle internal messages (like pong)
                msg = rx_internal.recv() => {
                    match msg {
                        Some(msg) => {
                            if let Ok(json_msg) = serde_json::to_string(&msg) {
                                if sender.send(axum::extract::ws::Message::Text(json_msg.into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    });

    // If any task finishes, abort the other
    tokio::pin!(send_task);
    tokio::pin!(recv_task);

    tokio::select! {
        _ = &mut send_task => {
            recv_task.abort();
        },
        _ = &mut recv_task => {
            send_task.abort();
        }
    }
}

async fn home_page(State(state): State<AppState>) -> impl IntoResponse {
    let history = state.get_history();
    let display_history = prepare_rate_display(&history);
    let template = IndexTemplate {
        history: display_history,
    };
    match template.render() {
        Ok(html) => Html(html),
        Err(e) => {
            eprintln!("Template render error: {}", e);
            Html("<h1>Template Error</h1>".to_string())
        }
    }
}

async fn api_latest(State(state): State<AppState>) -> impl IntoResponse {
    match state.get_latest_rate() {
        Some(rate) => (StatusCode::OK, axum::Json(rate)).into_response(),
        None => (StatusCode::NOT_FOUND, "No rate data available").into_response(),
    }
}

async fn api_history(State(state): State<AppState>) -> impl IntoResponse {
    let history = state.get_history();
    (StatusCode::OK, axum::Json(history))
}

fn prepare_rate_display(history: &[RateRecord]) -> Vec<RateDisplay> {
    history
        .iter()
        .enumerate()
        .map(|(index, record)| {
            let (change_text, change_indicator) = if index < history.len() - 1 {
                let previous_rate: f64 = history[index + 1].rate.parse().unwrap_or(0.0);
                let current_rate: f64 = record.rate.parse().unwrap_or(0.0);
                let change = current_rate - previous_rate;

                let indicator = if change > 0.0 {
                    "<span class='trend-up'>↗</span>".to_string()
                } else if change < 0.0 {
                    "<span class='trend-down'>↘</span>".to_string()
                } else {
                    "<span class='trend-flat'>→</span>".to_string()
                };

                let change_text = if change > 0.0 {
                    format!("<span class='change-up'>+{:.2}</span>", change)
                } else if change < 0.0 {
                    format!("<span class='change-down'>{:.2}</span>", change)
                } else {
                    "<span class='change-flat'>0.00</span>".to_string()
                };

                (change_text, indicator)
            } else {
                (String::new(), String::new())
            };

            RateDisplay {
                rate: record.rate.clone(),
                timestamp: record.timestamp,
                change_text,
                change_indicator,
            }
        })
        .collect()
}

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
    // Initialize database
    let db = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:rates.db".to_string());
    println!("Database URL: {}", db);
    let db_pool = SqlitePool::connect(&db).await.unwrap();

    // Run migrations
    sqlx::migrate!("./migrations").run(&db_pool).await.unwrap();

    let state = AppState::new(db_pool);

    // Load existing data from database
    if let Err(e) = state.load_from_db().await {
        eprintln!("⚠️ 加载历史数据失败: {}", e);
    }

    // Start the rate fetcher in the background
    let fetcher_state = state.clone();
    tokio::spawn(async move {
        rate_fetcher(fetcher_state).await;
    });

    // Start the periodic persister in the background
    let persister_state = state.clone();
    tokio::spawn(async move {
        periodic_persister(persister_state).await;
    });

    let server_state = state.clone();
    tokio::spawn(async move {
        // Build the router
        let app = Router::new()
            .route("/", get(home_page))
            .route("/ws", get(websocket_handler))
            .route("/api/latest", get(api_latest))
            .route("/api/history", get(api_history))
            .layer(ServiceBuilder::new().layer(CorsLayer::permissive()))
            .with_state(server_state);

        // Start the server
        let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
            .await
            .unwrap();

        println!("🚀 Server running on http://127.0.0.1:3000");
        println!("🔌 WebSocket endpoint: ws://127.0.0.1:3000/ws");

        axum::serve(listener, app).await.unwrap();
    });

    wait_for_shutdown_signal().await;
    state.persist_to_db().await.unwrap();
}
