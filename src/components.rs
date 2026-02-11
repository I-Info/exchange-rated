use crate::models::{
    Candle, CibRateRecord, IndicatorPoint, MacdRsiSeries, RateRecord, RateRecordWithLocalTime,
};

use chrono::{DateTime, Duration, Local, TimeZone, Utc};
use dioxus::prelude::*;

const FAVICON: Asset = asset!("/assets/favicon.ico");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");
const APEXCHART_CSS: Asset = asset!("/node_modules/apexcharts/dist/apexcharts.css");
const APEXCHART_JS: Asset = asset!("/node_modules/apexcharts/dist/apexcharts.min.js");
const LODASH_JS: Asset = asset!("/node_modules/lodash/lodash.min.js");

#[derive(Clone, Debug)]
struct RenderedRecord {
    rate: String,
    update_time: String,
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq)]
enum ConnectionStatus {
    Connecting,
    Connected,
    Closed,
    Error,
}

impl ConnectionStatus {
    fn badge_class(&self) -> &'static str {
        match self {
            ConnectionStatus::Connecting => "badge badge-warning",
            ConnectionStatus::Connected => "badge badge-success",
            ConnectionStatus::Closed => "badge badge-info",
            ConnectionStatus::Error => "badge badge-error",
        }
    }

    fn text(&self) -> &'static str {
        match self {
            ConnectionStatus::Connecting => "Connecting",
            ConnectionStatus::Connected => "Live",
            ConnectionStatus::Closed => "Closed",
            ConnectionStatus::Error => "Error",
        }
    }
}

impl PartialEq for RenderedRecord {
    fn eq(&self, other: &Self) -> bool {
        self.update_time == other.update_time
    }
}

#[component]
pub fn App() -> Element {
    rsx!(
        document::Link { rel: "icon", href: FAVICON }
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Link { rel: "stylesheet", href: APEXCHART_CSS }
        script { src: LODASH_JS }
        script { src: APEXCHART_JS }
        main { class: "@container flex min-h-screen flex-col", Main {} }

    )
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, serde::Serialize)]
struct ChartData {
    x: i64, // timestamp
    y: f64, // rate
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, serde::Serialize)]
struct CandleSeriesPoint {
    x: i64,
    y: [f64; 4],
}

#[cfg(feature = "web")]
const MAX_RETRIES: u32 = 10;

#[component]
pub fn ExtremeValues(
    highest: Option<RateRecordWithLocalTime>,
    lowest: Option<RateRecordWithLocalTime>,
) -> Element {
    let mut highest_rendered = use_signal(|| {
        highest.as_ref().map(|record| RenderedRecord {
            rate: record.rate.clone(),
            update_time: record.timestamp.format("%Y-%m-%d %H:%M:%S %Z").to_string(),
        })
    });

    let mut lowest_rendered = use_signal(|| {
        lowest.as_ref().map(|record| RenderedRecord {
            rate: record.rate.clone(),
            update_time: record.timestamp.format("%Y-%m-%d %H:%M:%S %Z").to_string(),
        })
    });

    #[cfg(feature = "web")]
    use_effect(move || {
        highest_rendered.set(highest.as_ref().map(|record| RenderedRecord {
            rate: record.rate.clone(),
            update_time: record.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
        }));
        lowest_rendered.set(lowest.as_ref().map(|record| RenderedRecord {
            rate: record.rate.clone(),
            update_time: record.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
        }));
    });

    rsx! {
        div { class: "my-4 grid grid-cols-1 gap-4 @sm:grid-cols-2",
            div { class: "card bg-base-100",
                div { class: "card-body text-center",
                    h4 { class: "justify-center card-title", "Recent High" }
                    if let Some(record) = highest_rendered() {
                        p { class: "text-2xl font-bold text-base-content", "{record.rate}" }
                        p { class: "text-xs text-base-content/60", "{record.update_time}" }
                    } else {
                        p { class: "text-2xl font-bold", "N/A" }
                    }
                }
            }
            div { class: "card bg-base-100",
                div { class: "card-body text-center",
                    h4 { class: "justify-center card-title", "Recent Low" }
                    if let Some(record) = lowest_rendered() {
                        p { class: "text-2xl font-bold text-base-content", "{record.rate}" }
                        p { class: "text-xs text-base-content/60", "{record.update_time}" }
                    } else {
                        p { class: "text-2xl font-bold", "N/A" }
                    }
                }
            }
        }
    }
}

#[component]
fn Main() -> Element {
    let records = use_server_future(get_rate)?.unwrap()?;
    let cib_records = use_server_future(get_cib_history)?.unwrap()?;

    #[allow(unused_mut)]
    let mut records_local: Signal<Vec<RateRecordWithLocalTime>> = use_signal(|| {
        records
            .iter()
            .map(|record| RateRecordWithLocalTime::from(record.clone()))
            .collect::<Vec<_>>()
    });

    let mut cib_latest: Signal<Option<CibRateRecord>> = use_signal(|| None);
    let mut cib_history: Signal<Vec<CibRateRecord>> = use_signal(|| cib_records.clone());

    let mut candle_interval_hours = use_signal(|| 4i64);
    let candles = use_server_future(move || get_candles(candle_interval_hours()))?;
    let indicators = use_server_future(move || get_indicators(candle_interval_hours()))?;

    #[allow(unused_mut)]
    let mut connection_status = use_signal(|| ConnectionStatus::Connecting);

    #[cfg(feature = "web")]
    use_future(move || async move {
        use crate::models::ServerEvent;
        use dioxus::logger::tracing;
        use futures::StreamExt;
        use gloo_net::eventsource::State;
        use gloo_net::eventsource::futures::EventSource;
        use gloo_timers::future::TimeoutFuture;
        use web_sys::wasm_bindgen::JsCast;

        let mut retry_count = 0u32;

        loop {
            if let Ok(mut es) = EventSource::new("/sse")
                && let Ok(mut stream) = es.subscribe("message")
            {
                // ES Connection checker
                let es_clone = es.clone();
                let task = spawn(async move {
                    loop {
                        match es_clone.state() {
                            State::Open => {
                                connection_status.set(ConnectionStatus::Connected);
                            }
                            State::Closed => {
                                connection_status.set(ConnectionStatus::Closed);
                                return;
                            }
                            State::Connecting => {
                                connection_status.set(ConnectionStatus::Connecting);
                            }
                        }
                        TimeoutFuture::new(1000).await;
                    }
                });

                // close es before unload
                let es_clone = es.clone();
                let on_unload = web_sys::wasm_bindgen::closure::Closure::once_into_js(move || {
                    connection_status.set(ConnectionStatus::Closed);
                    es_clone.close();
                });
                web_sys::window()
                    .unwrap()
                    .add_event_listener_with_callback("beforeunload", on_unload.unchecked_ref())
                    .unwrap();

                while let Some(Ok((_type, msg))) = stream.next().await {
                    connection_status.set(ConnectionStatus::Connected);
                    if let Some(raw) = msg.data().as_string() {
                        match serde_json::from_str::<ServerEvent>(&raw) {
                            Ok(ServerEvent::RateUpdate(record)) => {
                                let mut r = records_local.write();
                                r.push(record.into());
                                tracing::debug!("Rate Update received.");
                            }
                            Ok(ServerEvent::CibRateUpdate(record)) => {
                                let mut history = cib_history.write();
                                history.push(record.clone());
                                cib_latest.set(Some(record));
                                tracing::debug!("CIB rate update received.");
                            }
                            Ok(ServerEvent::History(records)) => {
                                records_local.set(
                                    records
                                        .into_iter()
                                        .map(RateRecordWithLocalTime::from)
                                        .collect::<Vec<_>>(),
                                );
                                tracing::debug!("History received.");
                            }
                            Ok(ServerEvent::CibHistory(records)) => {
                                if let Some(latest) = records.last().cloned() {
                                    cib_latest.set(Some(latest));
                                }
                                cib_history.set(records);
                                tracing::debug!("CIB history received.");
                            }
                            Err(err) => {
                                tracing::error!("Failed to parse server event: {}", err);
                                break;
                            }
                        }
                    }
                }

                task.cancel();
                if connection_status() == ConnectionStatus::Closed {
                    // Unloaded, abort
                    return;
                }
                es.close();
                connection_status.set(ConnectionStatus::Error);
                tracing::warn!("Stream being interrupted by some reason, closed.");
            } else {
                connection_status.set(ConnectionStatus::Error);
                tracing::error!(
                    "Failed to connect to event source, retry {} of {}",
                    retry_count + 1,
                    MAX_RETRIES
                );
            }

            retry_count += 1;
            if retry_count >= MAX_RETRIES {
                tracing::error!("Max reconnection attempts reached, giving up");
                connection_status.set(ConnectionStatus::Error);
                break;
            }

            // Exponential backoff with jitter: base delay of 500ms, max 30s
            let delay_ms = std::cmp::min(200 * 2u32.pow(retry_count), 30000);
            let jitter = (web_sys::js_sys::Math::random() * 0.1 + 0.95) as f64; // 10% jitter
            let final_delay = (delay_ms as f64 * jitter) as u32;
            tracing::info!("Reconnecting in {}ms...", final_delay);
            TimeoutFuture::new(final_delay).await;
        }
    });

    let mut current = use_signal(move || {
        records_local
            .read_unchecked()
            .last()
            .map(|record| RenderedRecord {
                rate: record.rate.clone(),
                update_time: record.timestamp.format("%Y-%m-%d %H:%M:%S %Z").to_string(),
            })
    });

    #[cfg(feature = "web")]
    use_effect(move || {
        current.set(
            records_local
                .read_unchecked()
                .last()
                .map(|record| RenderedRecord {
                    rate: record.rate.clone(),
                    update_time: record.timestamp.format("%Y-%m-%d %H:%M:%S").to_string(),
                }),
        );
    });

    let extremes = use_memo(move || {
        let records = records_local.read_unchecked();
        if records.is_empty() {
            return (None, None);
        };

        let mut high_rate = 0.0f64;
        let mut low_rate = f64::MAX;
        let mut high_index = 0;
        let mut low_index = 0;

        for (index, record) in records.iter().enumerate() {
            let rate = record.rate.parse::<f64>().unwrap();
            if rate > high_rate {
                high_rate = rate;
                high_index = index;
            }
            if rate < low_rate {
                low_rate = rate;
                low_index = index;
            }
        }

        let high_record = records.get(high_index).cloned();
        let low_record = records.get(low_index).cloned();

        (high_record, low_record)
    });

    // indicate rate delta
    let delta = use_memo(move || {
        if let Some(last) = records_local.last() {
            if let Some(prev) = records_local.get(records_local.len() - 2) {
                let delta = last.rate.parse::<f64>().unwrap() - prev.rate.parse::<f64>().unwrap();
                Some(delta)
            } else {
                None
            }
        } else {
            None
        }
    });

    rsx!(
        document::Title { "Rated" }
        div { class: "mx-auto w-full flex-1 px-4 @4xl:max-w-4xl",
            div { class: "card my-4 w-full",
                div { class: "card-body text-center",
                    div { class: "mb-4 flex items-center @sm:grid @sm:grid-cols-[1fr_auto_1fr]",
                        h3 { class: "text-left card-title @sm:col-start-2 @sm:text-center",
                            "Latest Rate"
                        }
                        span { class: "ml-auto @sm:col-start-3 @sm:ml-0 @sm:justify-self-end {connection_status.read().badge_class()}",
                            {connection_status.read_unchecked().text()}
                        }
                    }
                    {
                        match &*current.read_unchecked() {
                            Some(current) => {
                                rsx! {
                                    document::Title { "AUD/CNY: {current.rate}" }
                                    div { class: "grid grid-cols-[1fr_auto_1fr] items-center",
                                        span { class: "col-start-2 mx-2 text-4xl font-bold text-base-content", "{current.rate}" }
                                        {
                                            match *delta.read_unchecked() {
                                                Some(delta) => {
                                                    if delta > 0.0 {
                                                        rsx! {
                                                            span { class: "badge badge-success badge-soft", "{delta:+.2}" }
                                                        }
                                                    } else if delta < 0.0 {
                                                        rsx! {
                                                            span { class: "badge badge-error badge-soft", "{delta:.2}" }
                                                        }
                                                    } else {
                                                        rsx! {
                                                            span { class: "badge badge-soft badge-neutral", "{delta:.}" }
                                                        }
                                                    }
                                                }
                                                None => rsx! {},
                                            }
                                        }
                                    }
                                    p { class: "text-gray-500", "{current.update_time}" }
                                    div { class: "mt-2 text-sm text-base-content/70",
                                        span { class: "font-medium", "CIB参考价：" }
                                        if let Some(cib) = &*cib_latest.read_unchecked() {
                                            span { "{cib.rate}" }
                                            span { class: "ml-2 text-xs text-base-content/50", {cib.timestamp.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S").to_string()} }
                                        } else {
                                            span { "N/A" }
                                        }
                                    }
                                }
                            }
                            None => rsx! {
                                div { class: "text-4xl font-bold text-base-content", "No Rate Available" }
                                p { class: "text-gray-500", "Date: N/A" }
                            },
                        }
                    }
                }
            }
            {
                let (highest, lowest) = extremes();
                rsx! {
                    ExtremeValues { highest, lowest }
                }
            }
            Chart { records: records_local, cib_records: cib_history }
            div { class: "card my-4 w-full",
                div { class: "card-body",
                    div { class: "mb-4 flex flex-wrap items-center gap-2",
                        h3 { class: "card-title", "Candlestick" }
                        div { class: "ml-auto flex gap-2",
                            button {
                                class: if candle_interval_hours() == 4 { "btn btn-sm btn-primary" } else { "btn btn-sm btn-ghost" },
                                onclick: move |_| candle_interval_hours.set(4),
                                "4h"
                            }
                            button {
                                class: if candle_interval_hours() == 24 { "btn btn-sm btn-primary" } else { "btn btn-sm btn-ghost" },
                                onclick: move |_| candle_interval_hours.set(24),
                                "1d"
                            }
                        }
                    }
                    CandlestickChart { candles }
                }
            }
            div { class: "card my-4 w-full",
                div { class: "card-body",
                    h3 { class: "card-title", "Indicators" }
                    div { class: "grid gap-4 @md:grid-cols-2",
                        MacdChart { indicators }
                        RsiChart { indicators }
                    }
                }
            }
        }
        footer { class: "flex items-center gap-4 rounded-t-box bg-base-100/80 px-6 py-4 shadow-sm shadow-base-300/20",
            aside { class: "items-center",
                p { class: "flex gap-1 text-base-content/50",
                    a {
                        class: "link font-medium link-hover text-base-content",
                        href: "https://github.com/I-Info/exchange-rated",
                        "Rated "
                    }
                    "©2025"
                }
            }
            p { class: "ml-auto flex gap-1 text-base-content/50",
                span { class: "hidden @lg:inline", "Powered by " }
                a {
                    class: "link hidden @lg:inline link-hover text-base-content",
                    href: "https://dioxuslabs.com",
                    "@Dioxus"
                }
                span { class: "hidden @lg:inline", ", developed by " }
                span { class: "@lg:hidden", "by " }
                a {
                    class: "link inline link-hover text-base-content",
                    href: "https://github.com/I-Info",
                    "@IInfo"
                }
            }
        }
    )
}

#[component]
pub fn Chart(
    records: Signal<Vec<RateRecordWithLocalTime>>,
    cib_records: Signal<Vec<CibRateRecord>>,
) -> Element {
    let mut chart_element: Signal<Option<Event<MountedData>>> = use_signal(|| None);
    #[cfg(feature = "web")]
    let mut chart_instance: Signal<Option<web_sys::wasm_bindgen::JsValue>> = use_signal(|| None);

    #[cfg(feature = "web")]
    use_effect(move || {
        use dioxus::web::WebEventExt;
        use gloo_utils::format::JsValueSerdeExt;
        use web_sys::js_sys::{Array, Function, Reflect};
        use web_sys::wasm_bindgen::{JsCast, JsValue};

        let el = if let Some(event) = &*chart_element.read_unchecked() {
            event.as_web_event()
        } else {
            return;
        };

        let window = web_sys::window().unwrap();
        let apexcharts = Reflect::get(&window, &JsValue::from_str("ApexCharts")).unwrap();
        let apexcharts_fn: &Function = apexcharts.dyn_ref().unwrap();
        let param_array = Array::new();
        param_array.push(&el);
        param_array.push(
            &JsValue::from_serde(&serde_json::json!({
                "series": [],
                "chart": {
                    "type": "area",
                    "height": 500,
                    "zoom": {
                        "autoScaleYaxis": true
                    }
                },
                "xaxis": {
                    "type": "datetime",
                    "labels": {
                        "datetimeUTC": false,
                        "style": {
                            "colors": "var(--color-base-content)",
                            "fontSize": "13px"
                        }
                    }
                },
                "yaxis": {
                  "labels": {
                    "style": {
                      "colors": "var(--color-base-content)",
                      "fontSize": "13px"
                    }
                  }
                },
                "dataLabels": {
                    "enabled": false
                },
                "stroke": {
                    "curve": "smooth",
                    "width": 2,
                },
                "tooltip": {
                    "x": {
                        "format": "MMM dd HH:mm:ss"
                    },
                },
                "fill": {
                    "type": "gradient",
                    "gradient": {
                      "opacityFrom": 0.9,
                      "opacityTo": 0.1,
                    }
                }
            }))
            .unwrap(),
        );
        let instance = Reflect::construct(apexcharts_fn, &param_array).unwrap();

        let render_fn = Reflect::get(&instance, &JsValue::from_str("render")).unwrap();
        let render_fn: &Function = render_fn.dyn_ref().unwrap();
        render_fn.call0(&instance).unwrap();

        chart_instance.set(Some(instance));
    });

    #[cfg(feature = "web")]
    use_effect(move || {
        use gloo_utils::format::JsValueSerdeExt;
        use web_sys::{
            js_sys::{Function, Reflect},
            wasm_bindgen::{JsCast, JsValue},
        };

        let instance = if let Some(instance) = chart_instance() {
            instance
        } else {
            return;
        };

        let records = records
            .read_unchecked()
            .iter()
            .map(|record| ChartData {
                x: record.timestamp.timestamp_millis(),
                y: record.rate.parse::<f64>().unwrap_or(0.0),
            })
            .collect::<Vec<_>>();

        let cib_records = cib_records
            .read_unchecked()
            .iter()
            .map(|record| ChartData {
                x: record.timestamp.timestamp_millis(),
                y: record.rate.parse::<f64>().unwrap_or(0.0),
            })
            .collect::<Vec<_>>();

        let update_series_fn = Reflect::get(&instance, &JsValue::from_str("updateSeries")).unwrap();
        let update_series_fn: &Function = update_series_fn.dyn_ref().unwrap();

        let mut series = vec![serde_json::json!({ "data": records, "name": "Rate" })];
        if !cib_records.is_empty() {
            series.push(serde_json::json!({ "data": cib_records, "name": "CIB" }));
        }

        update_series_fn
            .call1(&instance, &JsValue::from_serde(&series).unwrap())
            .unwrap();
    });

    rsx! {
        div {
            id: "chart-container",
            class: "h-[515px]",
            onmounted: move |element| {
                chart_element.set(Some(element));
            },
        }
    }
}

#[component]
pub fn CandlestickChart(candles: Resource<ServerFnResult<Vec<Candle>>>) -> Element {
    let mut chart_element: Signal<Option<Event<MountedData>>> = use_signal(|| None);
    #[cfg(feature = "web")]
    let mut chart_instance: Signal<Option<web_sys::wasm_bindgen::JsValue>> = use_signal(|| None);

    #[cfg(feature = "web")]
    use_effect(move || {
        use dioxus::web::WebEventExt;
        use gloo_utils::format::JsValueSerdeExt;
        use web_sys::js_sys::{Array, Function, Reflect};
        use web_sys::wasm_bindgen::{JsCast, JsValue};

        let el = if let Some(event) = &*chart_element.read_unchecked() {
            event.as_web_event()
        } else {
            return;
        };

        let window = web_sys::window().unwrap();
        let apexcharts = Reflect::get(&window, &JsValue::from_str("ApexCharts")).unwrap();
        let apexcharts_fn: &Function = apexcharts.dyn_ref().unwrap();
        let param_array = Array::new();
        param_array.push(&el);
        param_array.push(
            &JsValue::from_serde(&serde_json::json!({
                "series": [],
                "chart": {
                    "type": "candlestick",
                    "height": 500,
                    "toolbar": { "show": false }
                },
                "xaxis": {
                    "type": "datetime",
                    "labels": {
                        "datetimeUTC": false,
                        "style": {
                            "colors": "var(--color-base-content)",
                            "fontSize": "13px"
                        }
                    }
                },
                "yaxis": {
                    "labels": {
                        "style": {
                            "colors": "var(--color-base-content)",
                            "fontSize": "13px"
                        }
                    }
                },
                "tooltip": {
                    "x": { "format": "MMM dd HH:mm" }
                }
            }))
            .unwrap(),
        );
        let instance = Reflect::construct(apexcharts_fn, &param_array).unwrap();

        let render_fn = Reflect::get(&instance, &JsValue::from_str("render")).unwrap();
        let render_fn: &Function = render_fn.dyn_ref().unwrap();
        render_fn.call0(&instance).unwrap();

        chart_instance.set(Some(instance));
    });

    #[cfg(feature = "web")]
    use_effect(move || {
        use gloo_utils::format::JsValueSerdeExt;
        use web_sys::{
            js_sys::{Function, Reflect},
            wasm_bindgen::{JsCast, JsValue},
        };

        let instance = if let Some(instance) = chart_instance() {
            instance
        } else {
            return;
        };

        let candles = match &*candles.read_unchecked() {
            Some(Ok(candles)) => candles.clone(),
            Some(Err(_)) | None => return,
        };

        let series = candles
            .iter()
            .map(|candle| CandleSeriesPoint {
                x: candle.start_timestamp.timestamp_millis(),
                y: [candle.open, candle.high, candle.low, candle.close],
            })
            .collect::<Vec<_>>();

        let update_series_fn = Reflect::get(&instance, &JsValue::from_str("updateSeries")).unwrap();
        let update_series_fn: &Function = update_series_fn.dyn_ref().unwrap();

        update_series_fn
            .call1(
                &instance,
                &JsValue::from_serde(&serde_json::json!([
                    { "data": series, "name": "Rate" }
                ]))
                .unwrap(),
            )
            .unwrap();
    });

    rsx! {
        div {
            id: "candlestick-chart-container",
            class: "h-[515px]",
            onmounted: move |element| {
                chart_element.set(Some(element));
            },
        }
    }
}

#[component]
pub fn MacdChart(indicators: Resource<ServerFnResult<MacdRsiSeries>>) -> Element {
    let mut chart_element: Signal<Option<Event<MountedData>>> = use_signal(|| None);
    #[cfg(feature = "web")]
    let mut chart_instance: Signal<Option<web_sys::wasm_bindgen::JsValue>> = use_signal(|| None);

    #[cfg(feature = "web")]
    use_effect(move || {
        use dioxus::web::WebEventExt;
        use gloo_utils::format::JsValueSerdeExt;
        use web_sys::js_sys::{Array, Function, Reflect};
        use web_sys::wasm_bindgen::{JsCast, JsValue};

        let el = if let Some(event) = &*chart_element.read_unchecked() {
            event.as_web_event()
        } else {
            return;
        };

        let window = web_sys::window().unwrap();
        let apexcharts = Reflect::get(&window, &JsValue::from_str("ApexCharts")).unwrap();
        let apexcharts_fn: &Function = apexcharts.dyn_ref().unwrap();
        let param_array = Array::new();
        param_array.push(&el);
        param_array.push(
            &JsValue::from_serde(&serde_json::json!({
                "series": [],
                "chart": {
                    "type": "line",
                    "height": 300,
                    "toolbar": { "show": false }
                },
                "stroke": { "width": [2, 2, 0] },
                "plotOptions": {
                    "bar": { "columnWidth": "60%" }
                },
                "dataLabels": { "enabled": false },
                "xaxis": {
                    "type": "datetime",
                    "labels": {
                        "datetimeUTC": false,
                        "style": {
                            "colors": "var(--color-base-content)",
                            "fontSize": "12px"
                        }
                    }
                },
                "yaxis": {
                    "labels": {
                        "style": {
                            "colors": "var(--color-base-content)",
                            "fontSize": "12px"
                        }
                    }
                },
                "tooltip": {
                    "x": { "format": "MMM dd HH:mm" }
                }
            }))
            .unwrap(),
        );
        let instance = Reflect::construct(apexcharts_fn, &param_array).unwrap();

        let render_fn = Reflect::get(&instance, &JsValue::from_str("render")).unwrap();
        let render_fn: &Function = render_fn.dyn_ref().unwrap();
        render_fn.call0(&instance).unwrap();

        chart_instance.set(Some(instance));
    });

    #[cfg(feature = "web")]
    use_effect(move || {
        use gloo_utils::format::JsValueSerdeExt;
        use web_sys::{
            js_sys::{Function, Reflect},
            wasm_bindgen::{JsCast, JsValue},
        };

        let instance = if let Some(instance) = chart_instance() {
            instance
        } else {
            return;
        };

        let series_data = match &*indicators.read_unchecked() {
            Some(Ok(series)) => series.clone(),
            Some(Err(_)) | None => return,
        };

        let macd_series = series_data
            .macd
            .iter()
            .map(|point| ChartData {
                x: point.timestamp.timestamp_millis(),
                y: point.value,
            })
            .collect::<Vec<_>>();

        let signal_series = series_data
            .signal
            .iter()
            .map(|point| ChartData {
                x: point.timestamp.timestamp_millis(),
                y: point.value,
            })
            .collect::<Vec<_>>();

        let histogram_series = series_data
            .histogram
            .iter()
            .map(|point| ChartData {
                x: point.timestamp.timestamp_millis(),
                y: point.value,
            })
            .collect::<Vec<_>>();

        let update_series_fn = Reflect::get(&instance, &JsValue::from_str("updateSeries")).unwrap();
        let update_series_fn: &Function = update_series_fn.dyn_ref().unwrap();

        update_series_fn
            .call1(
                &instance,
                &JsValue::from_serde(&serde_json::json!([
                    { "data": macd_series, "name": "MACD", "type": "line" },
                    { "data": signal_series, "name": "Signal", "type": "line" },
                    { "data": histogram_series, "name": "Histogram", "type": "column" }
                ]))
                .unwrap(),
            )
            .unwrap();
    });

    rsx! {
        div {
            id: "macd-chart-container",
            class: "h-[300px]",
            onmounted: move |element| {
                chart_element.set(Some(element));
            },
        }
    }
}

#[component]
pub fn RsiChart(indicators: Resource<ServerFnResult<MacdRsiSeries>>) -> Element {
    let mut chart_element: Signal<Option<Event<MountedData>>> = use_signal(|| None);
    #[cfg(feature = "web")]
    let mut chart_instance: Signal<Option<web_sys::wasm_bindgen::JsValue>> = use_signal(|| None);

    #[cfg(feature = "web")]
    use_effect(move || {
        use dioxus::web::WebEventExt;
        use gloo_utils::format::JsValueSerdeExt;
        use web_sys::js_sys::{Array, Function, Reflect};
        use web_sys::wasm_bindgen::{JsCast, JsValue};

        let el = if let Some(event) = &*chart_element.read_unchecked() {
            event.as_web_event()
        } else {
            return;
        };

        let window = web_sys::window().unwrap();
        let apexcharts = Reflect::get(&window, &JsValue::from_str("ApexCharts")).unwrap();
        let apexcharts_fn: &Function = apexcharts.dyn_ref().unwrap();
        let param_array = Array::new();
        param_array.push(&el);
        param_array.push(
            &JsValue::from_serde(&serde_json::json!({
                "series": [],
                "chart": {
                    "type": "line",
                    "height": 300,
                    "toolbar": { "show": false }
                },
                "dataLabels": { "enabled": false },
                "xaxis": {
                    "type": "datetime",
                    "labels": {
                        "datetimeUTC": false,
                        "style": {
                            "colors": "var(--color-base-content)",
                            "fontSize": "12px"
                        }
                    }
                },
                "yaxis": {
                    "min": 0,
                    "max": 100,
                    "labels": {
                        "style": {
                            "colors": "var(--color-base-content)",
                            "fontSize": "12px"
                        }
                    }
                },
                "annotations": {
                    "yaxis": [
                        { "y": 70, "borderColor": "var(--color-base-content)" },
                        { "y": 30, "borderColor": "var(--color-base-content)" }
                    ]
                },
                "tooltip": {
                    "x": { "format": "MMM dd HH:mm" }
                }
            }))
            .unwrap(),
        );
        let instance = Reflect::construct(apexcharts_fn, &param_array).unwrap();

        let render_fn = Reflect::get(&instance, &JsValue::from_str("render")).unwrap();
        let render_fn: &Function = render_fn.dyn_ref().unwrap();
        render_fn.call0(&instance).unwrap();

        chart_instance.set(Some(instance));
    });

    #[cfg(feature = "web")]
    use_effect(move || {
        use gloo_utils::format::JsValueSerdeExt;
        use web_sys::{
            js_sys::{Function, Reflect},
            wasm_bindgen::{JsCast, JsValue},
        };

        let instance = if let Some(instance) = chart_instance() {
            instance
        } else {
            return;
        };

        let series_data = match &*indicators.read_unchecked() {
            Some(Ok(series)) => series.clone(),
            Some(Err(_)) | None => return,
        };

        let rsi_series = series_data
            .rsi
            .iter()
            .map(|point| ChartData {
                x: point.timestamp.timestamp_millis(),
                y: point.value,
            })
            .collect::<Vec<_>>();

        let update_series_fn = Reflect::get(&instance, &JsValue::from_str("updateSeries")).unwrap();
        let update_series_fn: &Function = update_series_fn.dyn_ref().unwrap();

        update_series_fn
            .call1(
                &instance,
                &JsValue::from_serde(&serde_json::json!([
                    { "data": rsi_series, "name": "RSI" }
                ]))
                .unwrap(),
            )
            .unwrap();
    });

    rsx! {
        div {
            id: "rsi-chart-container",
            class: "h-[300px]",
            onmounted: move |element| {
                chart_element.set(Some(element));
            },
        }
    }
}

#[server]
async fn get_rate() -> ServerFnResult<Vec<RateRecord>> {
    let axum::Extension(state): axum::Extension<crate::server::models::AppState> =
        FullstackContext::extract().await?;

    Ok(state.get_history())
}

#[server]
async fn get_cib_history() -> ServerFnResult<Vec<CibRateRecord>> {
    let axum::Extension(state): axum::Extension<crate::server::models::AppState> =
        FullstackContext::extract().await?;

    Ok(state.get_cib_history())
}

#[server]
async fn get_candles(interval_hours: i64) -> ServerFnResult<Vec<Candle>> {
    let axum::Extension(state): axum::Extension<crate::server::models::AppState> =
        FullstackContext::extract().await?;

    let mut records = state.get_history();
    records.sort_by_key(|record| record.timestamp);

    let interval_hours = interval_hours.max(1);
    let interval = Duration::hours(interval_hours);
    let interval_secs = interval.num_seconds();

    let mut candles = Vec::new();

    let mut current_start: Option<DateTime<Utc>> = None;
    let mut open = 0.0;
    let mut high = f64::MIN;
    let mut low = f64::MAX;
    let mut close = 0.0;

    for record in records {
        let rate = match record.rate.parse::<f64>() {
            Ok(rate) => rate,
            Err(_) => continue,
        };
        let ts = record.timestamp.timestamp();
        let bucket_ts = ts - (ts % interval_secs);
        let bucket_start = Utc.timestamp_opt(bucket_ts, 0).single().unwrap();

        if current_start
            .map(|start| start != bucket_start)
            .unwrap_or(true)
        {
            if let Some(start) = current_start {
                candles.push(Candle {
                    open,
                    high,
                    low,
                    close,
                    start_timestamp: start,
                    end_timestamp: start + interval,
                });
            }
            current_start = Some(bucket_start);
            open = rate;
            high = rate;
            low = rate;
            close = rate;
        } else {
            if rate > high {
                high = rate;
            }
            if rate < low {
                low = rate;
            }
            close = rate;
        }
    }

    if let Some(start) = current_start {
        candles.push(Candle {
            open,
            high,
            low,
            close,
            start_timestamp: start,
            end_timestamp: start + interval,
        });
    }

    Ok(candles)
}

#[server]
async fn get_indicators(interval_hours: i64) -> ServerFnResult<MacdRsiSeries> {
    let axum::Extension(state): axum::Extension<crate::server::models::AppState> =
        FullstackContext::extract().await?;

    let mut records = state.get_history();
    records.sort_by_key(|record| record.timestamp);

    let interval_hours = interval_hours.max(1);
    let interval = Duration::hours(interval_hours);
    let interval_secs = interval.num_seconds();

    let mut candles = Vec::new();

    let mut current_start: Option<DateTime<Utc>> = None;
    let mut open = 0.0;
    let mut high = f64::MIN;
    let mut low = f64::MAX;
    let mut close = 0.0;

    for record in records {
        let rate = match record.rate.parse::<f64>() {
            Ok(rate) => rate,
            Err(_) => continue,
        };
        let ts = record.timestamp.timestamp();
        let bucket_ts = ts - (ts % interval_secs);
        let bucket_start = Utc.timestamp_opt(bucket_ts, 0).single().unwrap();

        if current_start
            .map(|start| start != bucket_start)
            .unwrap_or(true)
        {
            if let Some(start) = current_start {
                candles.push(Candle {
                    open,
                    high,
                    low,
                    close,
                    start_timestamp: start,
                    end_timestamp: start + interval,
                });
            }
            current_start = Some(bucket_start);
            open = rate;
            high = rate;
            low = rate;
            close = rate;
        } else {
            if rate > high {
                high = rate;
            }
            if rate < low {
                low = rate;
            }
            close = rate;
        }
    }

    if let Some(start) = current_start {
        candles.push(Candle {
            open,
            high,
            low,
            close,
            start_timestamp: start,
            end_timestamp: start + interval,
        });
    }

    let closes = candles
        .iter()
        .map(|candle| (candle.end_timestamp, candle.close))
        .collect::<Vec<_>>();
    let close_values = closes.iter().map(|(_, value)| *value).collect::<Vec<_>>();

    fn ema(values: &[f64], period: usize) -> Vec<Option<f64>> {
        let mut out = vec![None; values.len()];
        if values.len() < period || period == 0 {
            return out;
        }

        let sum: f64 = values.iter().take(period).sum();
        let mut prev = sum / period as f64;
        out[period - 1] = Some(prev);

        let alpha = 2.0 / (period as f64 + 1.0);
        for i in period..values.len() {
            let next = alpha * values[i] + (1.0 - alpha) * prev;
            out[i] = Some(next);
            prev = next;
        }

        out
    }

    fn rsi(values: &[f64], period: usize) -> Vec<Option<f64>> {
        let mut out = vec![None; values.len()];
        if values.len() <= period || period == 0 {
            return out;
        }

        let mut gain = 0.0;
        let mut loss = 0.0;
        for i in 1..=period {
            let diff = values[i] - values[i - 1];
            if diff >= 0.0 {
                gain += diff;
            } else {
                loss += -diff;
            }
        }

        let mut avg_gain = gain / period as f64;
        let mut avg_loss = loss / period as f64;

        let rs = if avg_loss == 0.0 {
            f64::INFINITY
        } else {
            avg_gain / avg_loss
        };
        out[period] = Some(100.0 - (100.0 / (1.0 + rs)));

        for i in (period + 1)..values.len() {
            let diff = values[i] - values[i - 1];
            let current_gain = if diff > 0.0 { diff } else { 0.0 };
            let current_loss = if diff < 0.0 { -diff } else { 0.0 };

            avg_gain = (avg_gain * (period as f64 - 1.0) + current_gain) / period as f64;
            avg_loss = (avg_loss * (period as f64 - 1.0) + current_loss) / period as f64;

            let rs = if avg_loss == 0.0 {
                f64::INFINITY
            } else {
                avg_gain / avg_loss
            };
            out[i] = Some(100.0 - (100.0 / (1.0 + rs)));
        }

        out
    }

    let ema12 = ema(&close_values, 12);
    let ema26 = ema(&close_values, 26);

    let mut macd_vals = vec![None; close_values.len()];
    for i in 0..close_values.len() {
        if let (Some(fast), Some(slow)) = (ema12[i], ema26[i]) {
            macd_vals[i] = Some(fast - slow);
        }
    }

    let mut macd_compact = Vec::new();
    let mut macd_index = Vec::new();
    for (i, value) in macd_vals.iter().enumerate() {
        if let Some(val) = value {
            macd_compact.push(*val);
            macd_index.push(i);
        }
    }

    let signal_compact = ema(&macd_compact, 9);
    let mut signal_vals = vec![None; close_values.len()];
    for (pos, idx) in macd_index.iter().enumerate() {
        if let Some(val) = signal_compact[pos] {
            signal_vals[*idx] = Some(val);
        }
    }

    let mut histogram_vals = vec![None; close_values.len()];
    for i in 0..close_values.len() {
        if let (Some(macd), Some(signal)) = (macd_vals[i], signal_vals[i]) {
            histogram_vals[i] = Some(macd - signal);
        }
    }

    let rsi_vals = rsi(&close_values, 14);

    let mut macd = Vec::new();
    let mut signal = Vec::new();
    let mut histogram = Vec::new();
    let mut rsi = Vec::new();

    for i in 0..close_values.len() {
        let timestamp = closes[i].0;
        if let Some(value) = macd_vals[i] {
            macd.push(IndicatorPoint { timestamp, value });
        }
        if let Some(value) = signal_vals[i] {
            signal.push(IndicatorPoint { timestamp, value });
        }
        if let Some(value) = histogram_vals[i] {
            histogram.push(IndicatorPoint { timestamp, value });
        }
        if let Some(value) = rsi_vals[i] {
            rsi.push(IndicatorPoint { timestamp, value });
        }
    }

    Ok(MacdRsiSeries {
        macd,
        signal,
        histogram,
        rsi,
    })
}
