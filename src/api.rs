use std::convert::Infallible;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, Sse},
        IntoResponse,
    },
    routing::{get, post},
    Json, Router,
};
use tokio_stream::{wrappers::BroadcastStream, StreamExt};
use tower_http::services::ServeDir;

use crate::{comms, state::AppState};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/devices", get(list_devices))
        .route("/devices/stream", get(device_stream))
        .route("/devices/search", post(search_devices))
        .route("/devices/{id}/status", get(device_status))
        .route("/devices/{id}/speed", post(set_speed))
        .route("/devices/{id}/nickname", post(set_nickname))
        .with_state(state)
        .fallback_service(ServeDir::new("static"))
}

/// GET /devices — list all known devices (used for initial page load)
async fn list_devices(State(state): State<AppState>) -> impl IntoResponse {
    let reg = state.registry.lock().await;
    let devices: Vec<_> = reg.values().collect();
    Json(serde_json::json!(devices))
}

/// GET /devices/stream — SSE stream; pushes a device JSON object whenever one is updated
async fn device_stream(
    State(state): State<AppState>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.event_tx.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| match result {
        Ok(json) => Some(Ok(Event::default().data(json))),
        Err(_) => None, // lagged message, skip
    });
    Sse::new(stream)
}

/// POST /devices/search — broadcast discovery, returns count of new devices found
async fn search_devices(State(state): State<AppState>) -> impl IntoResponse {
    match comms::discover_devices(&state, 2000).await {
        Ok(count) => (StatusCode::OK, Json(serde_json::json!({ "found": count }))),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// GET /devices/{id}/status — fetch live status for a device
async fn device_status(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match comms::fetch_status(&state, &id).await {
        Ok(()) => {
            let reg = state.registry.lock().await;
            match reg.get(&id) {
                Some(device) => (StatusCode::OK, Json(serde_json::json!(device))),
                None => (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({ "error": "device not found" })),
                ),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// POST /devices/{id}/speed — set fan speed (1–3, or 255 for manual mode)
async fn set_speed(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(speed) = body["speed"].as_u64().map(|v| v as u8) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "missing or invalid 'speed' field (expected 1-3)" })),
        );
    };

    if speed == 0 || (speed > 3 && speed != 255) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "speed must be 1-3 (or 255 for manual mode)" })),
        );
    }

    match comms::set_speed(&state, &id, speed).await {
        Ok(()) => {
            let reg = state.registry.lock().await;
            match reg.get(&id) {
                Some(device) => (StatusCode::OK, Json(serde_json::json!(device))),
                None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "device not found" }))),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

/// POST /devices/{id}/nickname — set or clear a human-readable name for a device
async fn set_nickname(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let nickname: Option<String> = body["nickname"]
        .as_str()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // Update registry and nicknames map together.
    {
        let mut reg = state.registry.lock().await;
        let Some(device) = reg.get_mut(&id) else {
            return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "device not found" })));
        };
        device.nickname = nickname.clone();
        if let Ok(json) = serde_json::to_string(device) {
            let _ = state.event_tx.send(json);
        }
    }

    let mut nicknames = state.nicknames.lock().await;
    match &nickname {
        Some(name) => { nicknames.insert(id, name.clone()); }
        None => { nicknames.remove(&id); }
    }
    crate::persist::save_nicknames(&nicknames, &state.config.nicknames_file);

    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

// Tests

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn make_request(method: &str, uri: &str) -> axum::http::Request<Body> {
        axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn list_devices_empty_registry_returns_empty_array() {
        let app = router(crate::state::new_app_state(Default::default(), Default::default()));
        let response = app.oneshot(make_request("GET", "/devices")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn list_devices_returns_seeded_device() {
        use std::net::{IpAddr, Ipv4Addr};
        let state = crate::state::new_app_state(Default::default(), Default::default());
        {
            let mut reg = state.registry.lock().await;
            reg.insert("dev-01".to_string(), crate::state::Device {
                id: "dev-01".to_string(),
                ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
                nickname: None,
                last_status: None,
            });
        }
        let response = router(state).oneshot(make_request("GET", "/devices")).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        let arr = json.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], "dev-01");
        assert_eq!(arr[0]["ip"], "192.168.1.10");
    }

    #[tokio::test]
    async fn device_status_unknown_id_returns_500() {
        let app = router(crate::state::new_app_state(Default::default(), Default::default()));
        let response = app
            .oneshot(make_request("GET", "/devices/nonexistent/status"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = body_json(response.into_body()).await;
        assert!(json["error"].as_str().is_some());
    }
}
