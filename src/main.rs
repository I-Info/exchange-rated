use rated::components::App;

fn main() {
    dioxus::logger::init(dioxus::logger::tracing::Level::INFO).unwrap();

    #[cfg(feature = "web")]
    // Hydrate the application on the client
    dioxus::launch(App);

    // Launch axum on the server
    #[cfg(feature = "server")]
    {
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(rated::server::launch_server(App));
    }
}
