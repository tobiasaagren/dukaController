use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use tokio::net::UdpSocket;

use crate::{
    protocol::{create_search_packet, create_set_speed_packet, create_status_packet, parse_response, DEFAULT_PASSWORD},
    state::{Device, Registry, UdpLock},
};

const DUKA_PORT: u16 = 4000;
const BROADCAST_ADDR: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 255);

/// Broadcast a search packet and collect any responding devices into the registry.
/// Acquires `udp_lock` for the duration so no other operation can bind port 4000 simultaneously.
pub async fn discover_devices(registry: &Registry, udp_lock: &UdpLock, timeout_ms: u64) -> std::io::Result<usize> {
    let _guard = udp_lock.lock().await;

    let socket = UdpSocket::bind("0.0.0.0:4000").await?;
    socket.set_broadcast(true)?;

    let packet = create_search_packet();
    let target = SocketAddr::new(IpAddr::V4(BROADCAST_ADDR), DUKA_PORT);
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
                        let mut reg = registry.lock().await;
                        reg.entry(id.clone()).or_insert_with(|| Device {
                            id,
                            ip: peer.ip(),
                            last_status: None,
                        });
                        found += 1;
                    }
                }
            }
        }
    }

    Ok(found)
}

/// Fetch and store status for a single device by ID.
pub async fn fetch_status(registry: &Registry, udp_lock: &UdpLock, device_id: &str) -> std::io::Result<()> {
    let ip = {
        let reg = registry.lock().await;
        reg.get(device_id).map(|d| d.ip)
    };

    let Some(ip) = ip else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "device not in registry"));
    };

    let _guard = udp_lock.lock().await;

    let socket = UdpSocket::bind("0.0.0.0:4000").await?;
    let target = SocketAddr::new(ip, DUKA_PORT);
    let packet = create_status_packet(device_id, DEFAULT_PASSWORD);
    socket.send_to(&packet, target).await?;

    let mut buf = vec![0u8; 1024];
    let (len, _) = tokio::time::timeout(
        tokio::time::Duration::from_secs(2),
        socket.recv_from(&mut buf),
    )
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "no response"))??;

    if let Some(status) = parse_response(&buf[..len], device_id.to_string()) {
        let mut reg = registry.lock().await;
        if let Some(device) = reg.get_mut(device_id) {
            device.last_status = Some(status);
        }
    }

    Ok(())
}

/// Send a speed command to a device, then immediately refresh its status.
pub async fn set_speed(registry: &Registry, udp_lock: &UdpLock, device_id: &str, speed: u8) -> std::io::Result<()> {
    let ip = {
        let reg = registry.lock().await;
        reg.get(device_id).map(|d| d.ip)
    };

    let Some(ip) = ip else {
        return Err(std::io::Error::new(std::io::ErrorKind::NotFound, "device not in registry"));
    };

    {
        let _guard = udp_lock.lock().await;
        let socket = UdpSocket::bind("0.0.0.0:4000").await?;
        let packet = create_set_speed_packet(device_id, DEFAULT_PASSWORD, speed);
        socket.send_to(&packet, SocketAddr::new(ip, DUKA_PORT)).await?;
        // Give the device a moment to apply the change before we read back status.
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    fetch_status(registry, udp_lock, device_id).await
}

/// Fetch status for every device currently in the registry, sequentially.
pub async fn refresh_all_statuses(registry: &Registry, udp_lock: &UdpLock) {
    let ids: Vec<String> = registry.lock().await.keys().cloned().collect();
    for id in ids {
        if let Err(e) = fetch_status(registry, udp_lock, &id).await {
            eprintln!("Status error for {id}: {e}");
        }
    }
}
