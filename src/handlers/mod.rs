use askama::Template;
use axum::{
    extract::{State, WebSocketUpgrade},
    http::StatusCode,
    response::{Html, IntoResponse},
    Json,
};
use futures_util::{sink::SinkExt, stream::StreamExt};

use crate::{AppState, WebSocketMessage, prepare_rate_display};

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    history: Vec<crate::models::RateDisplay>,
}

pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: axum::extract::ws::WebSocket, state: AppState) {
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
                    if let Ok(WebSocketMessage::Ping) =
                        serde_json::from_str::<WebSocketMessage>(&text)
                    {
                        let pong = WebSocketMessage::Pong;
                        if tx.send(pong).is_err() {
                            break;
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

pub async fn home_page(State(state): State<AppState>) -> impl IntoResponse {
    let history = state.get_history();
    let display_history = prepare_rate_display(&history);
    let template = IndexTemplate {
        history: display_history,
    };
    match template.render() {
        Ok(html) => Html(html),
        Err(e) => {
            eprintln!("Template render error: {e}");
            Html("<h1>Template Error</h1>".to_string())
        }
    }
}

pub async fn api_latest(State(state): State<AppState>) -> impl IntoResponse {
    match state.get_latest_rate() {
        Some(rate) => (StatusCode::OK, Json(rate)).into_response(),
        None => (StatusCode::NOT_FOUND, "No rate data available").into_response(),
    }
}

pub async fn api_history(State(state): State<AppState>) -> impl IntoResponse {
    let history = state.get_history();
    (StatusCode::OK, Json(history))
}