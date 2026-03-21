use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use tower_http::services::ServeDir;

use crate::{comms, state::AppState};

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/devices", get(list_devices))
        .route("/devices/search", post(search_devices))
        .route("/devices/{id}/status", get(device_status))
        .route("/devices/{id}/speed", post(set_speed))
        .with_state(state)
        .fallback_service(ServeDir::new("static"))
}

/// GET /devices — list all known devices
async fn list_devices(State(state): State<AppState>) -> impl IntoResponse {
    let reg = state.registry.lock().await;
    let devices: Vec<_> = reg.values().collect();
    Json(serde_json::json!(devices))
}

/// POST /devices/search — broadcast discovery, returns count of new devices found
async fn search_devices(State(state): State<AppState>) -> impl IntoResponse {
    match comms::discover_devices(&state.registry, &state.udp_lock, 2000).await {
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
    match comms::fetch_status(&state.registry, &state.udp_lock, &id).await {
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

/// POST /devices/{id}/speed — set fan speed (1–6, or 255 for manual mode)
async fn set_speed(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let Some(speed) = body["speed"].as_u64().map(|v| v as u8) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "missing or invalid 'speed' field (expected 1–6)" })),
        );
    };

    if speed == 0 || (speed > 3 && speed != 255) {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": "speed must be 1–3 (or 255 for manual mode)" })),
        );
    }

    match comms::set_speed(&state.registry, &state.udp_lock, &id, speed).await {
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

// ── Tests ─────────────────────────────────────────────────────────────────────

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
        let app = router(crate::state::new_app_state());

        let response = app.oneshot(make_request("GET", "/devices")).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let json = body_json(response.into_body()).await;
        assert_eq!(json, serde_json::json!([]));
    }

    #[tokio::test]
    async fn list_devices_returns_seeded_device() {
        use std::net::{IpAddr, Ipv4Addr};
        let state = crate::state::new_app_state();
        {
            let mut reg = state.registry.lock().await;
            reg.insert("dev-01".to_string(), crate::state::Device {
                id: "dev-01".to_string(),
                ip: IpAddr::V4(Ipv4Addr::new(192, 168, 1, 10)),
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
        // fetch_status returns NotFound immediately for IDs not in the registry,
        // which the handler maps to a 500 with an error message.
        let app = router(crate::state::new_app_state());

        let response = app
            .oneshot(make_request("GET", "/devices/nonexistent/status"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let json = body_json(response.into_body()).await;
        assert!(json["error"].as_str().is_some());
    }
}
