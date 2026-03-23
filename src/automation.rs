use std::collections::HashMap;

use crate::{comms, config::AutomationConfig, state::AppState};

/// Background task: periodically fetches outdoor conditions and adjusts fan speeds
/// for any device that has automation configured (min_speed + max_speed set).
pub async fn run(state: AppState) {
    let cfg = state.config.clone();
    let fetch_interval = tokio::time::Duration::from_secs(cfg.automation.outdoor_fetch_interval_secs);

    let mut outdoor_conditions: Option<(f64, u8)> = None; // (temp_c, rh_percent)
    // Subtract fetch_interval so the first fetch happens on the very first iteration.
    let mut last_fetch = tokio::time::Instant::now() - fetch_interval;
    // Tracks when each device last had its speed raised by automation.
    let mut speed_raised_at: HashMap<String, tokio::time::Instant> = HashMap::new();
    let lockout = tokio::time::Duration::from_secs(cfg.automation.speed_decrease_lockout_secs);

    let mut interval = tokio::time::interval(
        tokio::time::Duration::from_secs(cfg.status_interval_secs),
    );
    interval.tick().await; // skip first tick — let discovery and status refresh run first

    loop {
        interval.tick().await;

        if last_fetch.elapsed() >= fetch_interval {
            outdoor_conditions = fetch_outdoor_conditions(
                cfg.automation.latitude,
                cfg.automation.longitude,
            )
            .await;
            match outdoor_conditions {
                Some((t, rh)) => println!("Automation: outdoor temp={t:.1}°C humidity={rh}%"),
                None => eprintln!("Automation: failed to fetch outdoor conditions"),
            }
            last_fetch = tokio::time::Instant::now();
        }

        let Some((outdoor_temp_c, outdoor_rh)) = outdoor_conditions else { continue };
        let outdoor_ah = absolute_humidity(outdoor_temp_c, outdoor_rh);

        let ids: Vec<String> = state.registry.lock().await.keys().cloned().collect();
        for id in ids {
            let device_info = {
                let reg = state.registry.lock().await;
                reg.get(&id).and_then(|d| {
                    if d.unreachable || !d.automation_enabled {
                        return None;
                    }
                    let min_speed = d.automation_min_speed?;
                    let max_speed = d.automation_max_speed?;
                    let s = d.last_status.as_ref()?;
                    if s.speed == 255 {
                        return None; // device is in manual speed mode
                    }
                    let indoor_temp_c = d.assumed_indoor_temp_c
                        .unwrap_or(cfg.automation.assumed_indoor_temp_c);
                    Some((s.speed, s.humidity, indoor_temp_c, min_speed, max_speed))
                })
            };

            let Some((current_speed, indoor_rh, indoor_temp_c, min_speed, max_speed)) = device_info else {
                continue;
            };

            let indoor_ah = absolute_humidity(indoor_temp_c, indoor_rh);

            let Some(target) =
                compute_target_speed(indoor_ah, outdoor_ah, &cfg.automation, min_speed, max_speed)
            else {
                continue;
            };

            if target > current_speed {
                println!(
                    "Automation: {id} speed {current_speed}→{target} \
                     (indoor AH={indoor_ah:.2} g/m³, outdoor AH={outdoor_ah:.2} g/m³)"
                );
                let _ = comms::set_speed(&state, &id, target).await;
                speed_raised_at.insert(id.clone(), tokio::time::Instant::now());
            } else if target < current_speed {
                let locked = speed_raised_at.get(&id)
                    .map(|t| t.elapsed() < lockout)
                    .unwrap_or(false);
                if locked {
                    continue;
                }
                println!(
                    "Automation: {id} speed {current_speed}→{target} \
                     (indoor AH={indoor_ah:.2} g/m³, outdoor AH={outdoor_ah:.2} g/m³)"
                );
                let _ = comms::set_speed(&state, &id, target).await;
            }
        }
    }
}

/// Compute absolute humidity in g/m³ from temperature (°C) and relative humidity (%).
/// Uses the Magnus formula approximation.
fn absolute_humidity(temp_c: f64, rh_percent: u8) -> f64 {
    let rh = rh_percent as f64;
    let e_sat = 6.112 * ((17.67 * temp_c) / (temp_c + 243.5)).exp();
    (e_sat * rh * 2.1674) / (273.15 + temp_c)
}

/// Compute the target fan speed from absolute humidity readings, clamped to [min_speed, max_speed].
/// Returns None if outdoor AH >= indoor AH (ventilating would increase indoor moisture).
fn compute_target_speed(
    indoor_ah: f64,
    outdoor_ah: f64,
    cfg: &AutomationConfig,
    min_speed: u8,
    max_speed: u8,
) -> Option<u8> {
    let delta = indoor_ah - outdoor_ah;
    let raw = if delta >= cfg.speed3_abs_humidity_delta {
        3u8
    } else if delta >= cfg.speed2_abs_humidity_delta {
        2
    } else if delta >= cfg.speed1_abs_humidity_delta {
        1
    } else {
        return None; // outdoor is at least as humid — don't automate
    };
    Some(raw.clamp(min_speed, max_speed))
}

async fn fetch_outdoor_conditions(lat: f64, lon: f64) -> Option<(f64, u8)> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast\
         ?latitude={lat}&longitude={lon}&current=temperature_2m,relative_humidity_2m"
    );
    let response = reqwest::get(&url).await.ok()?;
    let json: serde_json::Value = response.json().await.ok()?;
    let temp_c = json["current"]["temperature_2m"].as_f64()?;
    let rh = json["current"]["relative_humidity_2m"].as_u64().map(|v| v as u8)?;
    Some((temp_c, rh))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(s1: f64, s2: f64, s3: f64) -> AutomationConfig {
        AutomationConfig {
            latitude: 0.0,
            longitude: 0.0,
            outdoor_fetch_interval_secs: 300,
            assumed_indoor_temp_c: 20.0,
            speed1_abs_humidity_delta: s1,
            speed2_abs_humidity_delta: s2,
            speed3_abs_humidity_delta: s3,
        }
    }

    #[test]
    fn below_threshold_returns_none() {
        // outdoor AH > indoor AH
        assert_eq!(compute_target_speed(8.0, 9.0, &cfg(0.0, 1.5, 3.0), 1, 3), None);
    }

    #[test]
    fn at_speed1_threshold() {
        // delta == 0.0, at speed1 threshold
        assert_eq!(compute_target_speed(8.0, 8.0, &cfg(0.0, 1.5, 3.0), 1, 3), Some(1));
    }

    #[test]
    fn at_speed2_threshold() {
        assert_eq!(compute_target_speed(9.5, 8.0, &cfg(0.0, 1.5, 3.0), 1, 3), Some(2));
    }

    #[test]
    fn at_speed3_threshold() {
        assert_eq!(compute_target_speed(11.0, 8.0, &cfg(0.0, 1.5, 3.0), 1, 3), Some(3));
    }

    #[test]
    fn clamped_to_max_speed() {
        // delta = 4.0, would be speed 3, but max is 2
        assert_eq!(compute_target_speed(12.0, 8.0, &cfg(0.0, 1.5, 3.0), 1, 2), Some(2));
    }

    #[test]
    fn clamped_to_min_speed() {
        // delta = 0.5, would be speed 1, but min is 2
        assert_eq!(compute_target_speed(8.5, 8.0, &cfg(0.0, 1.5, 3.0), 2, 3), Some(2));
    }

    #[test]
    fn absolute_humidity_reasonable_values() {
        // At 20°C, 50% RH ≈ 8.65 g/m³
        let ah = absolute_humidity(20.0, 50);
        assert!((ah - 8.65).abs() < 0.1, "got {ah}");
    }
}
