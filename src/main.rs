use axum::{
    Router,
    extract::{State, WebSocketUpgrade, ws::WebSocket},
    http::StatusCode,
    response::{Html, IntoResponse},
    routing::get,
};
use chrono::{DateTime, Local};
use futures_util::{sink::SinkExt, stream::StreamExt};
use regex::Regex;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::sleep;
use tower::ServiceBuilder;
use tower_http::cors::CorsLayer;

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
    timestamp: DateTime<Local>,
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
    client: Client,
    broadcast_tx: broadcast::Sender<WebSocketMessage>,
}

impl AppState {
    fn new() -> Self {
        let (broadcast_tx, _) = broadcast::channel(1000);

        Self {
            rate_history: Arc::new(RwLock::new(VecDeque::new())),
            client: Client::builder().user_agent("Mozilla/5.0").build().unwrap(),
            broadcast_tx,
        }
    }

    fn add_rate(&self, rate: String) {
        let mut history = self.rate_history.write().unwrap();

        let record = RateRecord {
            rate,
            timestamp: Local::now(),
        };

        // Check if this rate is different from the last one
        if let Some(last) = history.back() {
            if last.rate == record.rate {
                return; // Skip if rate hasn't changed
            }
        }

        println!(
            "[{}] 澳元汇卖价更新: {}",
            record.timestamp.format("%Y-%m-%d %H:%M:%S"),
            record.rate
        );

        history.push_back(record.clone());

        // Keep only the last 100 records
        while history.len() > 100 {
            history.pop_front();
        }

        // Broadcast the update to all connected WebSocket clients
        let message = WebSocketMessage::RateUpdate { record };
        let _ = self.broadcast_tx.send(message);
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
}

async fn fetch_sell_rate(client: &Client) -> Option<String> {
    let html = client.get(URL).send().await.ok()?.text().await.ok()?;

    // 匹配 "澳大利亚元" 行后的两个 <td>，第2个是卖出价
    if let Some(caps) = &RE.captures(&html) {
        return Some(caps.get(2)?.as_str().trim().to_string()); // 卖出价
    }

    None
}

async fn rate_fetcher(state: AppState) {
    let interval = Duration::from_secs(3);

    loop {
        match fetch_sell_rate(&state.client).await {
            Some(current_rate) => {
                state.add_rate(current_rate);
            }
            None => {
                eprintln!("无法获取牌价！");
            }
        }

        sleep(interval).await;
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
    let html = generate_html(&history);
    Html(html)
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

fn generate_html(history: &[RateRecord]) -> String {
    let latest_display = match history.first() {
        Some(rate) => format!(
            "<div class='latest-rate'>
                <h2>最新汇率</h2>
                <div class='rate-value'>{}</div>
                <div class='timestamp'>更新时间: {}</div>
            </div>",
            rate.rate,
            rate.timestamp.format("%Y/%m/%d %H:%M:%S")
        ),
        None => "<div class='latest-rate'><h2>系统初始化中，暂无数据，请手动刷新。</h2></div>"
            .to_string(),
    };

    let history_rows = history
        .iter()
        .map(|record| {
            format!(
                "<tr>
                    <td>{}</td>
                    <td>{}</td>
                </tr>",
                record.rate,
                record.timestamp.format("%Y/%m/%d %H:%M:%S")
            )
        })
        .collect::<Vec<_>>()
        .join("");

    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>澳元汇率监控</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            max-width: 1200px;
            margin: 0 auto;
            padding: 20px;
            background-color: #f5f5f5;
        }}
        .container {{
            background: white;
            border-radius: 10px;
            padding: 30px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }}
        h1 {{
            color: #333;
            text-align: center;
            margin-bottom: 30px;
        }}
        .latest-rate {{
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 30px;
            border-radius: 10px;
            text-align: center;
            margin-bottom: 30px;
        }}
        .rate-value {{
            font-size: 3em;
            font-weight: bold;
            margin: 20px 0;
        }}
        .timestamp {{
            font-size: 0.9em;
            opacity: 0.8;
        }}
        .history-section {{
            margin-top: 30px;
        }}
        .history-section h2 {{
            color: #333;
            margin-bottom: 20px;
        }}
        table {{
            width: 100%;
            border-collapse: collapse;
            margin-top: 20px;
        }}
        th, td {{
            padding: 12px;
            text-align: left;
            border-bottom: 1px solid #ddd;
        }}
        th {{
            background-color: #f8f9fa;
            font-weight: bold;
            color: #333;
        }}
        tr:hover {{
            background-color: #f8f9fa;
        }}
        .connection-status {{
            text-align: center;
            margin-top: 20px;
            padding: 10px;
            border-radius: 5px;
            font-weight: bold;
        }}
        .connected {{
            background-color: #d4edda;
            color: #155724;
        }}
        .disconnected {{
            background-color: #f8d7da;
            color: #721c24;
        }}
        .connecting {{
            background-color: #fff3cd;
            color: #856404;
        }}
        .api-links {{
            margin-top: 20px;
            text-align: center;
        }}
        .api-links a {{
            color: #667eea;
            text-decoration: none;
            margin: 0 10px;
        }}
        .api-links a:hover {{
            text-decoration: underline;
        }}
        .refresh-info {{
            text-align: center;
            margin-top: 20px;
            color: #666;
            font-size: 0.9em;
        }}
    </style>
    <script>
        let ws;
        let reconnectTimeout;
        let isReconnecting = false;

        function updateConnectionStatus(status) {{
            const statusElement = document.getElementById('connection-status');
            statusElement.className = 'connection-status ' + status;

            switch(status) {{
                case 'connected':
                    statusElement.textContent = '🟢 WebSocket 已连接 - 实时更新';
                    break;
                case 'disconnected':
                    statusElement.textContent = '🔴 WebSocket 已断开 - 正在重连...';
                    break;
                case 'connecting':
                    statusElement.textContent = '🟡 正在连接 WebSocket...';
                    break;
            }}
        }}

        function updateLatestRate(record) {{
            document.querySelector('.rate-value').textContent = record.rate;
            document.querySelector('.timestamp').textContent = '更新时间: ' + new Date(record.timestamp).toLocaleString('zh-CN');
        }}

        function updateHistory(records) {{
            const tbody = document.querySelector('table tbody');
            tbody.innerHTML = records.map(record => {{
                const timestamp = new Date(record.timestamp).toLocaleString('zh-CN');
                return `<tr><td>${{record.rate}}</td><td>${{timestamp}}</td></tr>`;
            }}).join('');

            // Update record count
            document.querySelector('.history-section h2').textContent = `汇率历史 (最近${{records.length}}条记录)`;
        }}

        function connectWebSocket() {{
            if (isReconnecting) return;

            updateConnectionStatus('connecting');

            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            const wsUrl = `${{protocol}}//${{window.location.host}}/ws`;

            ws = new WebSocket(wsUrl);

            ws.onopen = function() {{
                console.log('WebSocket connected');
                updateConnectionStatus('connected');
                isReconnecting = false;

                // Clear any existing reconnect timeout
                if (reconnectTimeout) {{
                    clearTimeout(reconnectTimeout);
                    reconnectTimeout = null;
                }}
            }};

            ws.onmessage = function(event) {{
                try {{
                    const data = JSON.parse(event.data);

                    switch(data.type) {{
                        case 'rate_update':
                            updateLatestRate(data.record);
                            // Add the new record to the beginning of the history
                            const tbody = document.querySelector('table tbody');
                            const newRow = document.createElement('tr');
                            const timestamp = new Date(data.record.timestamp).toLocaleString('zh-CN');
                            newRow.innerHTML = `<td>${{data.record.rate}}</td><td>${{timestamp}}</td>`;
                            tbody.insertBefore(newRow, tbody.firstChild);

                            // Remove last row if we have more than 100 records
                            const rows = tbody.querySelectorAll('tr');
                            if (rows.length > 100) {{
                                tbody.removeChild(rows[rows.length - 1]);
                            }}

                            // Update record count
                            document.querySelector('.history-section h2').textContent = `汇率历史 (最近${{rows.length}}条记录)`;
                            break;

                        case 'history':
                            updateHistory(data.records);
                            if (data.records.length > 0) {{
                                updateLatestRate(data.records[0]);
                            }}
                            break;

                        case 'pong':
                            console.log('Received pong');
                            break;
                    }}
                }} catch (error) {{
                    console.error('Error parsing WebSocket message:', error);
                }}
            }};

            ws.onclose = function() {{
                console.log('WebSocket disconnected');
                updateConnectionStatus('disconnected');

                // Attempt to reconnect after 3 seconds
                if (!isReconnecting) {{
                    isReconnecting = true;
                    reconnectTimeout = setTimeout(() => {{
                        isReconnecting = false;
                        connectWebSocket();
                    }}, 3000);
                }}
            }};

            ws.onerror = function(error) {{
                console.error('WebSocket error:', error);
                updateConnectionStatus('disconnected');
            }};
        }}

        // Send periodic ping to keep connection alive
        function sendPing() {{
            if (ws && ws.readyState === WebSocket.OPEN) {{
                ws.send(JSON.stringify({{type: 'ping'}}));
            }}
        }}

        // Connect when page loads
        document.addEventListener('DOMContentLoaded', function() {{
            connectWebSocket();

            // Send ping every 30 seconds to keep connection alive
            setInterval(sendPing, 30000);
        }});

        // Handle page visibility changes
        document.addEventListener('visibilitychange', function() {{
            if (!document.hidden && ws && ws.readyState === WebSocket.CLOSED) {{
                connectWebSocket();
            }}
        }});
    </script>
</head>
<body>
    <div class="container">
        <h1>澳元汇率监控 (AUD/CNY)</h1>

        <div id="connection-status" class="connection-status connecting">
            🟡 正在连接 WebSocket...
        </div>

        {}

        <div class="history-section">
            <h2>汇率历史 (最近{}条记录)</h2>
            <table>
                <thead>
                    <tr>
                        <th>汇率</th>
                        <th>时间</th>
                    </tr>
                </thead>
                <tbody>
                    {}
                </tbody>
            </table>
        </div>

        <div class="api-links">
            <strong>API 接口:</strong>
            <a href="/api/latest">最新汇率</a>
            <a href="/api/history">完整历史</a>
            <a href="/ws">WebSocket 连接</a>
        </div>

        <div class="refresh-info">
            实时 WebSocket 更新 | 数据每3秒更新一次
        </div>
    </div>
</body>
</html>"#,
        latest_display,
        history.len(),
        history_rows
    )
}

#[tokio::main]
async fn main() {
    let state = AppState::new();

    // Start the rate fetcher in the background
    let fetcher_state = state.clone();
    tokio::spawn(async move {
        rate_fetcher(fetcher_state).await;
    });

    // Build the router
    let app = Router::new()
        .route("/", get(home_page))
        .route("/ws", get(websocket_handler))
        .route("/api/latest", get(api_latest))
        .route("/api/history", get(api_history))
        .layer(ServiceBuilder::new().layer(CorsLayer::permissive()))
        .with_state(state);

    // Start the server
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .unwrap();

    println!("🚀 Server running on http://127.0.0.1:3000");
    println!("🔌 WebSocket endpoint: ws://127.0.0.1:3000/ws");
    println!("📊 Rate fetcher starting...");

    axum::serve(listener, app).await.unwrap();
}
