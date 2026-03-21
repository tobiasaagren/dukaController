mod api;
mod comms;
mod config;
mod persist;
mod protocol;
mod state;

#[tokio::main]
async fn main() {
    let cfg = config::load_config();
    let discovery_interval = cfg.discovery_interval_secs;
    let status_interval = cfg.status_interval_secs;
    let nicknames_file = cfg.nicknames_file.clone();
    let app_state = state::new_app_state(cfg, persist::load_nicknames(&nicknames_file));

    // Background task: discover devices at startup, then every DISCOVERY_INTERVAL_SECS.
    let s = app_state.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(discovery_interval));
        loop {
            interval.tick().await; // first tick fires immediately
            match comms::discover_devices(&s, 2000).await {
                Ok(n) => println!("Discovery: {n} device(s) found"),
                Err(e) => eprintln!("Discovery error: {e}"),
            }
        }
    });

    // Background task: refresh status for all known devices every STATUS_INTERVAL_SECS.
    let s = app_state.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(status_interval));
        interval.tick().await; // skip the immediate first tick — let discovery run first
        loop {
            interval.tick().await;
            comms::refresh_all_statuses(&s).await;
        }
    });

    let app = api::router(app_state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    println!("Listening on http://0.0.0.0:3000");
    axum::serve(listener, app).await.unwrap();
}
