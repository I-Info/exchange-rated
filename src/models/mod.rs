use std::sync::{Arc, RwLock};
use std::{collections::VecDeque};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RateRecord {
    pub rate: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Clone, Debug)]
pub struct ExtremeValues {
    pub high_rate: f64,
    pub low_rate: f64,
    pub high_timestamp: DateTime<Utc>,
    pub low_timestamp: DateTime<Utc>,
}

pub enum Extreme {
    High(f64, DateTime<Utc>),
    Low(f64, DateTime<Utc>),
}

#[derive(Clone, Debug)]
pub struct NtfyConfig {
    pub url: String,
    pub auth: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RateDisplay {
    pub rate: String,
    pub timestamp: DateTime<Utc>,
    pub change_text: String,
    pub change_indicator: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WebSocketMessage {
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
pub struct AppState {
    pub rate_history: Arc<RwLock<VecDeque<RateRecord>>>,
    pub broadcast_tx: broadcast::Sender<WebSocketMessage>,
    pub extreme_values: Arc<RwLock<Option<ExtremeValues>>>,
    pub db_pool: sqlx::SqlitePool,
}

#[derive(Clone, Debug)]
pub struct CacheInfo {
    // etag: String,
    pub last_modified: String,
}