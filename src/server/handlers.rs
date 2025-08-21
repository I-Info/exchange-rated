use super::models::AppState;
use crate::models::ServerEvent;

use axum::{
    extract::{State, WebSocketUpgrade, ws},
    response::{
        IntoResponse, Sse,
        sse::{Event, KeepAlive},
    },
};
use futures::{SinkExt, StreamExt, stream};
use tokio_stream::wrappers::BroadcastStream;

pub async fn websocket_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, state))
}

async fn handle_socket(socket: ws::WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();
    let mut rx = state.subscribe();

    // Send initial history to the client
    let history = state.get_history();
    let history_message = ServerEvent::History(history);
    if let Ok(msg) = serde_json::to_string(&history_message)
        && sender.send(ws::Message::Text(msg.into())).await.is_err() {
            return;
        }

    // Handle incoming messages from client
    let recv_task = tokio::spawn(async move {
        while let Some(msg) = receiver.next().await {
            if let Ok(ws::Message::Close(_)) = msg {
                break;
            }
        }
    });

    // Handle broadcast messages and internal messages to client
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if let Ok(json_msg) = serde_json::to_string(&msg)
                && sender
                    .send(ws::Message::Text(json_msg.into()))
                    .await
                    .is_err()
                {
                    break;
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

pub async fn sse_handler(
    State(state): State<AppState>,
) -> Sse<impl futures::Stream<Item = Result<Event, anyhow::Error>>> {
    let history = state.get_history();
    let rx = state.subscribe();
    let stream = stream::once(async { Ok(ServerEvent::History(history)) })
        .chain(BroadcastStream::from(rx))
        .map(|event| match event {
            Ok(event) => Ok(Event::default().json_data(event)?),
            Err(err) => Err(anyhow::Error::new(err)),
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// pub async fn home_page(State(state): State<AppState>) -> impl IntoResponse {
//     let history = state.get_history();
//     let display_history = prepare_rate_display(&history);
//     let template = IndexTemplate {
//         history: display_history,
//     };
//     match template.render() {
//         Ok(html) => Html(html),
//         Err(e) => {
//             eprintln!("Template render error: {e}");
//             Html("<h1>Template Error</h1>".to_string())
//         }
//     }
// }

// pub async fn api_latest(State(state): State<AppState>) -> impl IntoResponse {
//     match state.get_latest_rate() {
//         Some(rate) => (StatusCode::OK, Json(rate)).into_response(),
//         None => (StatusCode::NOT_FOUND, "No rate data available").into_response(),
//     }
// }

// pub async fn api_history(State(state): State<AppState>) -> impl IntoResponse {
//     let history = state.get_history();
//     (StatusCode::OK, Json(history))
// }
