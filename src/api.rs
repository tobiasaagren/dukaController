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

use crate::{auth, comms, state::AppState};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/login", get(auth::login_get).post(auth::login_post))
        .route("/logout", post(auth::logout_post))
        .route("/devices", get(list_devices))
        .route("/devices/stream", get(device_stream))
        .route("/devices/search", post(search_devices))
        .route("/devices/{id}/status", get(device_status))
        .route("/devices/{id}/speed", post(set_speed))
        .route("/devices/{id}/nickname", post(set_nickname))
        .route("/devices/{id}/mode", post(set_mode))
        .route("/devices/{id}/automation", post(set_automation))
        .route("/outdoor", get(get_outdoor_conditions))
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

/// POST /devices/{id}/mode — set ventilation mode (one_way | two_way | in)
async fn set_mode(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let mode = match body["mode"].as_str() {
        Some("one_way") => crate::protocol::DeviceMode::OneWay,
        Some("two_way") => crate::protocol::DeviceMode::TwoWay,
        Some("in")      => crate::protocol::DeviceMode::In,
        _ => return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "mode must be one of: one_way, two_way, in" })),
        ),
    };

    match comms::set_mode(&state, &id, mode).await {
        Ok(()) => {
            let reg = state.registry.lock().await;
            match reg.get(&id) {
                Some(device) => (StatusCode::OK, Json(serde_json::json!(device))),
                None => (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "device not found" }))),
            }
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({ "error": e.to_string() }))),
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

    let mut settings = state.settings.lock().await;
    settings.entry(id).or_default().nickname = nickname;
    crate::persist::save_settings(&settings, &state.config.settings_file);

    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

/// POST /devices/{id}/automation — set per-device automation speed range.
/// Body: `{ "min_speed": 1, "max_speed": 3 }` to enable; omit either field to clear.
async fn set_automation(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let automation_enabled = body["enabled"].as_bool().unwrap_or(false);
    let min_speed = body["min_speed"].as_u64().map(|v| v as u8);
    let max_speed = body["max_speed"].as_u64().map(|v| v as u8);
    let assumed_indoor_temp_c = body["assumed_indoor_temp_c"].as_f64();

    for &speed in [min_speed, max_speed].iter().flatten() {
        if !(1..=3).contains(&speed) {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "speed must be 1–3" })),
            );
        }
    }
    if let (Some(min), Some(max)) = (min_speed, max_speed) {
        if min > max {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": "min_speed must be <= max_speed" })),
            );
        }
    }

    {
        let mut reg = state.registry.lock().await;
        let Some(device) = reg.get_mut(&id) else {
            return (StatusCode::NOT_FOUND, Json(serde_json::json!({ "error": "device not found" })));
        };
        device.automation_enabled = automation_enabled;
        device.automation_min_speed = min_speed;
        device.automation_max_speed = max_speed;
        device.assumed_indoor_temp_c = assumed_indoor_temp_c;
        if let Ok(json) = serde_json::to_string(device) {
            let _ = state.event_tx.send(json);
        }
    }

    let mut settings = state.settings.lock().await;
    let entry = settings.entry(id).or_default();
    entry.automation_enabled = automation_enabled;
    entry.automation_min_speed = min_speed;
    entry.automation_max_speed = max_speed;
    entry.assumed_indoor_temp_c = assumed_indoor_temp_c;
    crate::persist::save_settings(&settings, &state.config.settings_file);

    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

/// GET /outdoor — latest outdoor conditions fetched by the automation task
async fn get_outdoor_conditions(State(state): State<AppState>) -> impl IntoResponse {
    let cond = *state.outdoor_conditions.lock().await;
    match cond {
        Some((temp_c, rh)) => Json(serde_json::json!({ "temp_c": temp_c, "rh": rh })),
        None => Json(serde_json::json!(null)),
    }
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
                unreachable: false,
                consecutive_failures: 0,
                last_status: None,
                automation_enabled: false,
                automation_min_speed: None,
                automation_max_speed: None,
                assumed_indoor_temp_c: None,
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
