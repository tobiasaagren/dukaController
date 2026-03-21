mod api;
mod comms;
mod persist;
mod protocol;
mod state;

const DISCOVERY_INTERVAL_SECS: u64 = 30;
const STATUS_INTERVAL_SECS: u64 = 3;

#[tokio::main]
async fn main() {
    let app_state = state::new_app_state(persist::load_nicknames());

    // Background task: discover devices at startup, then every DISCOVERY_INTERVAL_SECS.
    let s = app_state.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(DISCOVERY_INTERVAL_SECS));
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
            tokio::time::interval(tokio::time::Duration::from_secs(STATUS_INTERVAL_SECS));
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
