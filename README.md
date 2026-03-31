# Duka

Humidity-based fan speed controller for Duka/Blauberg ventilation units on a local network. Communicates with devices over UDP using their proprietary binary protocol, exposes a REST/SSE API, and automatically adjusts fan speeds based on indoor vs. outdoor absolute humidity.

## Features

- **Auto-discovery** — broadcasts UDP to find devices on the network
- **Real-time monitoring** — SSE stream pushes device updates to all connected clients instantly
- **Humidity automation** — computes absolute humidity (g/m³) from local sensors and Open-Meteo weather data, sets fan speed based on configurable thresholds
- **Per-device overrides** — each device can have a custom nickname, speed range, and indoor temperature assumption
- **Web UI** — browser interface with session auth and IP-based rate limiting

## Requirements

- Rust (stable)
- Duka/Blauberg ventilation units reachable via UDP on your LAN

## Setup

1. Copy the example config and fill in your values:
   ```bash
   cp config.toml.example config.toml
   ```

2. Edit `config.toml` — at minimum set `username`, `password`, `broadcast_address`, and your `latitude`/`longitude` for weather data.

3. Build and run:
   ```bash
   cargo build --release
   ./target/release/duka
   ```

The server listens on `http://0.0.0.0:3000`.

## Configuration

See `config.toml.example` for all options with comments. Key sections:

| Section | Key settings |
|---|---|
| Auth | `username`, `password`, `session_ttl_secs` |
| Network | `broadcast_address`, `duka_port`, `device_password` |
| Task intervals | `discovery_interval_secs`, `status_interval_secs` |
| Automation | `latitude`, `longitude`, `speed1/2/3_abs_humidity_delta`, `speed_decrease_lockout_secs` |

Per-device settings (nicknames, min/max speed, assumed indoor temp) are persisted automatically to `settings.json`.

## API

All endpoints require an authenticated session cookie (log in via `POST /login`).

| Method | Path | Description |
|---|---|---|
| `GET` | `/devices` | List all known devices |
| `GET` | `/devices/stream` | SSE stream of real-time device updates |
| `POST` | `/devices/search` | Trigger discovery broadcast |
| `GET` | `/devices/{id}/status` | Fetch fresh device status |
| `POST` | `/devices/{id}/speed` | Set fan speed (`{"speed": 1-3}`) |
| `POST` | `/devices/{id}/mode` | Set ventilation mode (`one_way`, `two_way`, `in`) |
| `POST` | `/devices/{id}/nickname` | Set display name |
| `POST` | `/devices/{id}/automation` | Configure per-device automation |
| `GET` | `/outdoor` | Get cached outdoor conditions |

## Automation Logic

1. Fetches outdoor temperature and relative humidity from [Open-Meteo](https://open-meteo.com/) (cached hourly).
2. Converts RH + temperature to **absolute humidity** (g/m³) using the Magnus formula for both indoor and outdoor.
3. Computes delta = indoor AH − outdoor AH.
4. Maps the delta to a fan speed using configurable thresholds (`speed1/2/3_abs_humidity_delta`).
5. Clamps the result to each device's configured min/max speed range.
6. Applies a **speed-decrease lockout** after any speed increase to prevent rapid hunting.

## Development

```bash
cargo build        # Debug build
cargo test         # Run all tests
cargo clippy       # Lint
cargo fmt          # Format
```

## Docker

A `Dockerfile` is included. Mount your `config.toml` at runtime:

```bash
docker build -t duka .
docker run -p 3000:3000 -v $(pwd)/config.toml:/app/config.toml duka
```
