use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Form, Request, State},
    http::{header, HeaderMap, HeaderValue},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::state::AppState;

const MAX_ATTEMPTS: u32 = 5;
const COOKIE_NAME: &str = "duka_session";

fn generate_token() -> String {
    use rand::Rng;
    let bytes: [u8; 16] = rand::thread_rng().r#gen();
    bytes.map(|b| format!("{b:02x}")).concat()
}

fn session_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie.split(';').find_map(|part| {
        part.trim()
            .strip_prefix(&format!("{COOKIE_NAME}="))
            .map(str::to_string)
    })
}

pub async fn auth_middleware(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path.starts_with("/login") || path == "/logout" {
        return next.run(req).await;
    }
    let ttl = std::time::Duration::from_secs(state.config.session_ttl_secs);
    let authed = match session_from_headers(req.headers()) {
        Some(token) => state.sessions.lock().await
            .get(&token)
            .map(|created_at| created_at.elapsed() < ttl)
            .unwrap_or(false),
        None => false,
    };
    if authed {
        next.run(req).await
    } else {
        Redirect::to("/login").into_response()
    }
}

pub async fn logout_post(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    if let Some(token) = session_from_headers(&headers) {
        state.sessions.lock().await.remove(&token);
    }
    let mut response = Redirect::to("/login").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{COOKIE_NAME}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0; Secure"
        ))
        .expect("cookie value is always valid ASCII"),
    );
    response
}

pub async fn login_get() -> Html<&'static str> {
    Html(include_str!("../static/login.html"))
}

#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

pub async fn login_post(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: axum::http::HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    let ip = headers
        .get("CF-Connecting-IP")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| addr.ip());

    let attempts = *state.login_attempts.lock().await.get(&ip).unwrap_or(&0);
    if attempts >= MAX_ATTEMPTS {
        tracing::warn!(%ip, "login attempt while locked out");
        return Redirect::to("/login?error=locked").into_response();
    }

    if form.username == state.config.username && form.password == state.config.password {
        state.login_attempts.lock().await.remove(&ip);
        tracing::info!(%ip, "login success");
        let token = generate_token();
        state.sessions.lock().await.insert(token.clone(), std::time::Instant::now());
        let secure = if headers.contains_key("CF-Connecting-IP") { "; Secure" } else { "" };
        let cookie = format!("{COOKIE_NAME}={token}; HttpOnly; SameSite=Strict; Path=/{secure}");
        let mut response = Redirect::to("/").into_response();
        response.headers_mut().insert(
            header::SET_COOKIE,
            HeaderValue::from_str(&cookie).expect("cookie value is always valid ASCII"),
        );
        response
    } else {
        let mut map = state.login_attempts.lock().await;
        let count = map.entry(ip).or_insert(0);
        *count += 1;
        let locked = *count >= MAX_ATTEMPTS;
        drop(map);
        tracing::warn!(%ip, username = %form.username, attempt = attempts + 1, locked, "login failed");
        let error = if locked { "locked" } else { "invalid" };
        Redirect::to(&format!("/login?error={error}")).into_response()
    }
}
