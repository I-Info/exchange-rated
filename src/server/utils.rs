use super::models::NtfyConfig;

#[cfg(not(unix))]
use tokio::signal;
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};

pub async fn wait_for_shutdown_signal() {
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

pub fn load_ntfy_config() -> Option<NtfyConfig> {
    // Load email configuration from environment variables
    if let Ok(url) = std::env::var("NTFY_URL") {
        let auth = std::env::var("NTFY_AUTH");
        Some(NtfyConfig {
            url,
            auth: auth.ok(),
        })
    } else {
        None
    }
}
