use crate::models::RateRecord;

use dioxus::prelude::*;

// const FAVICON: Asset = asset!("/assets/favicon.ico");
const TAILWIND_CSS: Asset = asset!("/assets/tailwind.css");

#[component]
pub fn App() -> Element {
    let record = use_server_future(get_rate)?.unwrap()?;
    let mut timestamp =
        use_signal(move || record.timestamp.format("%Y-%m-%d %H:%M:%S UTC").to_string());
    use_effect(move || {
        timestamp.set(
            record
                .timestamp
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string(),
        );
    });

    rsx!(
        document::Link { rel: "stylesheet", href: TAILWIND_CSS }
        document::Title { "Latest Rate: {record.rate}" }
        main {
            div { class: "prose mx-auto w-96 shadow-sm text-center bg-base-100 border-base-300 border rounded-box",
                h2 { "Latest Rate" }
                    h1 { {record.rate} },
                    p { "Date: {timestamp}" }
            }
        }

    )
}

#[server]
async fn get_rate() -> ServerFnResult<RateRecord> {
    let FromContext::<crate::server::models::AppState>(state) = extract().await?;

    if let Some(rate) = state.get_latest_rate() {
        Ok(rate)
    } else {
        Err(ServerFnError::new("Rate not found"))
    }
}
