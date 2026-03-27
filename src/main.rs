mod api;
mod auth;
mod automation;
mod comms;
mod config;
mod persist;
mod protocol;
mod state;

async fn security_headers(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{header::HeaderName, HeaderValue};
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(HeaderName::from_static("x-content-type-options"),  HeaderValue::from_static("nosniff"));
    h.insert(HeaderName::from_static("x-frame-options"),         HeaderValue::from_static("DENY"));
    h.insert(HeaderName::from_static("referrer-policy"),         HeaderValue::from_static("same-origin"));
    h.insert(
        HeaderName::from_static("content-security-policy"),
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self' 'unsafe-inline'; \
             style-src 'self' 'unsafe-inline'; connect-src 'self'",
        ),
    );
    res
}

#[tokio::main]
async fn main() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let cfg = config::load_config();
    let discovery_interval = cfg.discovery_interval_secs;
    let status_interval = cfg.status_interval_secs;
    let settings_file = cfg.settings_file.clone();
    let app_state = state::new_app_state(cfg, persist::load_settings(&settings_file));

    // Background task: discover devices at startup, then every discovery_interval_secs.
    let s = app_state.clone();
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(discovery_interval));
        loop {
            interval.tick().await; // first tick fires immediately
            let _ = comms::discover_devices(&s, 2000).await;
        }
    });

    // Background task: refresh status for all known devices every status_interval_secs.
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

    // Background task: humidity-based fan speed automation.
    let s = app_state.clone();
    tokio::spawn(automation::run(s));

    let app = api::router(app_state.clone())
        .layer(axum::middleware::from_fn_with_state(
            app_state,
            auth::auth_middleware,
        ))
        .layer(axum::middleware::from_fn(security_headers))
        .layer(
            tower_http::trace::TraceLayer::new_for_http()
                .make_span_with(|req: &axum::http::Request<_>| {
                    let ip = req
                        .headers()
                        .get("CF-Connecting-IP")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            req.extensions()
                                .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
                                .map(|ci| ci.0.ip().to_string())
                                .unwrap_or_else(|| "-".to_string())
                        });
                    tracing::info_span!(
                        "request",
                        method = %req.method(),
                        path = req.uri().path(),
                        ip = %ip,
                    )
                })
                .on_response(
                    tower_http::trace::DefaultOnResponse::new()
                        .level(tracing::Level::INFO),
                ),
        );
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!("Listening on http://0.0.0.0:3000");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await
    .unwrap();
}
