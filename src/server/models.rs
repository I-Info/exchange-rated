use crate::models::{CibRateRecord, RateRecord, ServerEvent};

use std::collections::VecDeque;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use tokio::sync::broadcast;

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

#[derive(Clone)]
pub struct AppState {
    pub rate_history: Arc<RwLock<VecDeque<RateRecord>>>,
    pub cib_rate_history: Arc<RwLock<VecDeque<CibRateRecord>>>,
    pub broadcast_tx: broadcast::Sender<ServerEvent>,
    pub extreme_values: Arc<RwLock<Option<ExtremeValues>>>,
    pub db_pool: sqlx::SqlitePool,
}

#[derive(Clone, Debug)]
pub struct CacheInfo {
    // etag: String,
    pub last_modified: String,
}

#[derive(Clone, Debug)]
pub struct CibQuote {
    pub currency_name: String,
    pub currency_symbol: String,
    pub unit: String,
    pub spot_buy: String,
    pub spot_sell: String,
    pub cash_buy: String,
    pub cash_sell: String,
}

#[derive(Clone, Debug)]
pub struct CibQuoteSnapshot {
    pub as_of: DateTime<Utc>,
    pub quotes: Vec<CibQuote>,
}
