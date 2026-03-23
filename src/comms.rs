use std::net::{IpAddr, SocketAddr};
use tokio::net::UdpSocket;

use crate::{
    protocol::{create_search_packet, create_set_mode_packet, create_set_speed_packet, create_status_packet, parse_response, DeviceMode},
    state::{AppState, Device},
};

const UNREACHABLE_THRESHOLD: u32 = 20;

/// Broadcast a search packet and collect any responding devices into the registry.
pub async fn discover_devices(state: &AppState, timeout_ms: u64) -> std::io::Result<usize> {
    let _guard = state.udp_lock.lock().await;
    let cfg = &state.config;

    let socket = UdpSocket::bind(format!("0.0.0.0:{}", cfg.duka_port)).await?;
    socket.set_broadcast(true)?;

    let packet = create_search_packet();
    let target: SocketAddr = format!("{}:{}", cfg.broadcast_address, cfg.duka_port).parse()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    socket.send_to(&packet, target).await?;

    let deadline = tokio::time::Instant::now()
        + tokio::time::Duration::from_millis(timeout_ms);

    let mut buf = vec![0u8; 1024];
    let mut found = 0usize;

    loop {
        match tokio::time::timeout_at(deadline, socket.recv_from(&mut buf)).await {
            Err(_) => break,
            Ok(Err(e)) => return Err(e),
            Ok(Ok((len, peer))) => {
                let data = &buf[..len];
                if data.len() > 4 && data[0] == 0xFD && data[1] == 0xFD {
                    let id_len = data[3] as usize;
                    if data.len() >= 4 + id_len {
                        let id = String::from_utf8_lossy(&data[4..4 + id_len]).to_string();
                        if id == "DEFAULT_DEVICEID" { continue; }
                        let (nickname, automation_enabled, automation_min_speed, automation_max_speed, assumed_indoor_temp_c) = {
                            let s = state.settings.lock().await;
                            let entry = s.get(&id);
                            (
                                entry.and_then(|e| e.nickname.clone()),
                                entry.map(|e| e.automation_enabled).unwrap_or(false),
                                entry.and_then(|e| e.automation_min_speed),
                                entry.and_then(|e| e.automation_max_speed),
                                entry.and_then(|e| e.assumed_indoor_temp_c),
                            )
                        };
                        let mut reg = state.registry.lock().await;
                        let device = reg.entry(id.clone()).or_insert_with(|| Device {
                            id,
                            ip: peer.ip(),
                            nickname,
                            unreachable: false,
                            consecutive_failures: 0,
                            last_status: None,
                            automation_enabled,
                            automation_min_speed,
                            automation_max_speed,
                            assumed_indoor_temp_c,
                        });
                        if let Ok(json) = serde_json::to_string(device) {
                            let _ = state.event_tx.send(json);
                        }
                        found += 1;
                    }
                }
            }
        }
    }

    Ok(found)
}

/// Fetch and store status for a single device by ID, then push the update to SSE clients.
/// Tracks consecutive failures; marks device unreachable after UNREACHABLE_THRESHOLD failures.
pub async fn fetch_status(state: &AppState, device_id: &str) -> std::io::Result<()> {
    let ip = {
        let reg = state.registry.lock().await;
        reg.get(device_id).map(|d| d.ip)
    };

    let Some(ip) = ip else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "device not in registry"));
    };

    let cfg = &state.config;
    let result = {
        let _guard = state.udp_lock.lock().await;
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", cfg.duka_port)).await?;
        let target = SocketAddr::new(ip, cfg.duka_port);
        socket.send_to(&create_status_packet(device_id, &cfg.device_password), target).await?;

        let mut buf = vec![0u8; 1024];
        tokio::time::timeout(
            tokio::time::Duration::from_secs(2),
            socket.recv_from(&mut buf),
        )
        .await
        .ok()
        .and_then(|r| r.ok())
        .and_then(|(len, _)| parse_response(&buf[..len], device_id.to_string()))
    };

    let mut reg = state.registry.lock().await;
    let Some(device) = reg.get_mut(device_id) else { return Ok(()); };

    match result {
        Some(status) => {
            let was_unreachable = device.unreachable;
            device.last_status = Some(status);
            device.consecutive_failures = 0;
            device.unreachable = false;
            if let Ok(json) = serde_json::to_string(device) {
                let _ = state.event_tx.send(json);
            }
            if was_unreachable {
                println!("Device {device_id} is reachable again");
            }
            Ok(())
        }
        None => {
            device.consecutive_failures += 1;
            if device.consecutive_failures >= UNREACHABLE_THRESHOLD && !device.unreachable {
                device.unreachable = true;
                println!("Device {device_id} marked unreachable after {UNREACHABLE_THRESHOLD} failed attempts");
                if let Ok(json) = serde_json::to_string(device) {
                    let _ = state.event_tx.send(json);
                }
            }
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "no response"))
        }
    }
}

/// Send a speed command to a device, then immediately refresh its status.
pub async fn set_speed(state: &AppState, device_id: &str, speed: u8) -> std::io::Result<()> {
    let info = {
        let reg = state.registry.lock().await;
        reg.get(device_id).map(|d| (d.ip, d.unreachable))
    };
    let Some((ip, unreachable)) = info else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "device not in registry"));
    };
    if unreachable {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "device is unreachable"));
    }

    let cfg = &state.config;
    {
        let _guard = state.udp_lock.lock().await;
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", cfg.duka_port)).await?;
        let packet = create_set_speed_packet(device_id, &cfg.device_password, speed);
        socket.send_to(&packet, SocketAddr::new(ip, cfg.duka_port)).await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    fetch_status(state, device_id).await
}

/// Send a mode command to a device, then immediately refresh its status.
pub async fn set_mode(state: &AppState, device_id: &str, mode: DeviceMode) -> std::io::Result<()> {
    let info = {
        let reg = state.registry.lock().await;
        reg.get(device_id).map(|d| (d.ip, d.unreachable))
    };
    let Some((ip, unreachable)) = info else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "device not in registry"));
    };
    if unreachable {
        return Err(std::io::Error::new(std::io::ErrorKind::Other, "device is unreachable"));
    }

    let cfg = &state.config;
    {
        let _guard = state.udp_lock.lock().await;
        let socket = UdpSocket::bind(format!("0.0.0.0:{}", cfg.duka_port)).await?;
        let packet = create_set_mode_packet(device_id, &cfg.device_password, mode);
        socket.send_to(&packet, SocketAddr::new(ip, cfg.duka_port)).await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    fetch_status(state, device_id).await
}

/// Fetch status for every device currently in the registry, sequentially.
pub async fn refresh_all_statuses(state: &AppState) {
    let ids: Vec<String> = state.registry.lock().await.keys().cloned().collect();
    for id in ids {
        let _ = fetch_status(state, &id).await;
    }
}
