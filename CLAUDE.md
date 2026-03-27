# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build --release      # Production build
cargo build                # Debug build
cargo test                 # Run all tests
cargo test <test_name>     # Run a single test (e.g. cargo test test_calculate_absolute_humidity)
cargo clippy               # Lint
cargo fmt                  # Format
```

The app runs on port 3000 and requires a `config.toml` (see `config.toml.example`).

## Architecture

Duka is a **humidity-based fan controller** for Duka/Blauberg ventilation units on a local network. It communicates with devices over UDP using a proprietary binary protocol, exposes a REST/SSE API, and automates fan speeds based on indoor vs. outdoor absolute humidity.

### Module Overview

| Module | Role |
|---|---|
| `main.rs` | Entry point; spawns three background tasks (discovery, status polling, automation), starts Axum server |
| `protocol.rs` | Duka UDP binary protocol — packet construction, checksum, parsing |
| `comms.rs` | UDP device I/O — discovery, status fetch, speed/mode commands; uses a shared `udp_lock` to serialize socket access |
| `automation.rs` | Background task — fetches outdoor weather from Open-Meteo, computes absolute humidity, adjusts fan speeds |
| `api.rs` | Axum HTTP handlers — REST endpoints + SSE stream for real-time device updates |
| `auth.rs` | Session auth middleware — cookie-based sessions, IP rate limiting (5 attempts), Cloudflare-aware (`CF-Connecting-IP`) |
| `state.rs` | `AppState` — `Arc`-wrapped shared state: device registry, sessions, SSE broadcast sender, outdoor conditions cache |
| `config.rs` | Loads `config.toml`; falls back to hardcoded defaults |
| `persist.rs` | Loads/saves per-device settings (nicknames, automation bounds) to `settings.json` |

### Key Data Flows

**Discovery**: `main.rs` background task → `comms::discover_devices()` → UDP broadcast → upserts registry → broadcasts via `event_tx`

**Status polling**: background task → `comms::refresh_all_statuses()` → UDP per-device query → updates registry → SSE broadcast. Devices marked unreachable after 20 consecutive failures.

**Automation**: background task → fetch outdoor weather (cached) → compute absolute humidity delta per device → `comms::set_speed()` if threshold crossed. Speed-raise lockout prevents rapid decreases after an increase.

**Real-time UI**: `GET /devices/stream` (SSE) subscribes to `state.event_tx`; every registry mutation sends the updated `Device` JSON to all connected clients.

### Concurrency

- `Arc<Mutex<T>>` for registry, sessions, settings
- `tokio::sync::broadcast` (capacity 64) for SSE fan-out
- `udp_lock: Arc<Mutex<()>>` serializes all UDP socket operations

### Configuration

- `config.toml` — credentials, network (broadcast address, UDP port, device password), task intervals, automation lat/lon, humidity delta thresholds
- `settings.json` — persisted per-device overrides (nickname, automation min/max speed, assumed indoor temp); auto-created at runtime

Per-device automation settings override global `config.toml` values.
