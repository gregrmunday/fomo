use crate::model::*;
use chrono::{NaiveDateTime, Utc};
use serde_json::Value;
use std::process::Command;

// Published for development at https://coastalmonitoring.org/ccoresources/api/
pub const DEVELOPMENT_KEY: &str = "6cefd36d8e12a4dead4cf06d4dbd09c0";
pub const ATTRIBUTION: &str = "Real time data displayed on this page are from the Regional Coastal Monitoring Programme, made freely available under the terms of the Open Government Licence. Please note that these are real-time data and are not quality-controlled.";

fn get(kind: &str, sensor: Option<&str>, history: bool) -> Result<Value, String> {
    let key = std::env::var("FOMO_API_KEY").unwrap_or_else(|_| DEVELOPMENT_KEY.into());
    // The live service accepts its current website domain for the published key.
    // A personal key can specify its issued domain with FOMO_API_REFERER.
    let referer = std::env::var("FOMO_API_REFERER")
        .unwrap_or_else(|_| "https://coastalmonitoring.org/".into());
    let url = format!("https://coastalmonitoring.org/observations/{kind}/latest.geojson");
    // macOS supplies curl and its TLS stack. No embedded browser, async runtime,
    // or bundled TLS library is needed. This runs only on background threads.
    let mut cmd = Command::new("/usr/bin/curl");
    cmd.args([
        "--disable",
        "--silent",
        "--show-error",
        "--fail-with-body",
        "--proto",
        "=https",
        "--connect-timeout",
        "8",
        "--max-time",
        "20",
        "--max-filesize",
        "2097152",
        "--get",
    ])
    .arg("--header")
    .arg(format!("Referer: {referer}"))
    .args([
        "--header",
        "Accept: application/json",
        "--user-agent",
        "Fomo/0.1 (personal macOS buoy viewer)",
    ])
    .arg("--data-urlencode")
    .arg(format!("key={key}"));
    if let Some(sensor) = sensor {
        cmd.arg("--data-urlencode").arg(format!("sensor={sensor}"));
    }
    if history {
        cmd.args(["--data-urlencode", "duration=12"]);
    }
    let result = cmd
        .arg(url)
        .output()
        .map_err(|e| format!("Could not connect: {e}"))?;
    if !result.status.success() {
        let body = String::from_utf8_lossy(&result.stdout);
        return Err(
            if body.contains("key") || body.contains("Referer") || body.contains("Unauthorized") {
                "API key or Referer rejected. Check the API settings in README.".into()
            } else {
                "Could not reach Coastal Monitoring. Retrying automatically.".into()
            },
        );
    }
    let value: Value = serde_json::from_slice(&result.stdout)
        .map_err(|_| "The buoy service returned an unreadable response")?;
    if !value["features"].is_array() {
        return Err("The buoy service returned an unexpected response".into());
    }
    Ok(value)
}

fn number(p: &Value, key: &str, min: f64, max: f64) -> Option<f64> {
    let v = p[key].as_f64().or_else(|| p[key].as_str()?.parse().ok())?;
    (v.is_finite() && (min..=max).contains(&v)).then_some(v)
}
fn time(p: &Value) -> Option<chrono::DateTime<Utc>> {
    NaiveDateTime::parse_from_str(p["date"].as_str()?, "%Y%m%d#%H%M%S")
        .ok()
        .map(|d| d.and_utc())
}
fn flag(p: &Value) -> Option<u32> {
    number(p, "QCflag", 0.0, 99.0).map(|f| f as u32)
}

pub fn parse_waves(value: &Value, spot: Spot) -> Result<Vec<Wave>, String> {
    let mut waves: Vec<_> = value["features"]
        .as_array()
        .ok_or("Missing buoy observations")?
        .iter()
        .filter_map(|f| {
            let p = &f["properties"];
            if p["sensor"].as_str()? != spot.name() {
                return None;
            }
            let flag = flag(p);
            let height_bad = matches!(flag, Some(1 | 9));
            let period_bad = matches!(flag, Some(1 | 2 | 9));
            let direction_bad = matches!(flag, Some(1 | 2 | 3 | 6 | 9));
            Some(Wave {
                time: time(p)?,
                height: (!height_bad).then(|| number(p, "hs", 0.0, 40.0)).flatten(),
                period: (!period_bad).then(|| number(p, "tp", 0.1, 50.0)).flatten(),
                direction: (!direction_bad)
                    .then(|| number(p, "pdir", 0.0, 360.0))
                    .flatten(),
                temperature: (!matches!(flag, Some(8 | 9)))
                    .then(|| number(p, "sst", -5.0, 45.0))
                    .flatten(),
                flag,
            })
        })
        .collect();
    waves.sort_by_key(|w| w.time);
    waves.dedup_by_key(|w| w.time);
    if waves.is_empty() {
        Err(format!("No observations returned for {}", spot.name()))
    } else {
        Ok(waves)
    }
}

fn parse_wind(value: &Value) -> Vec<Wind> {
    value["features"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let p = &f["properties"];
            let flag = flag(p);
            let xy = f["geometry"]["coordinates"].as_array()?;
            Some(Wind {
                time: time(p)?,
                station: p["sensor"].as_str()?.into(),
                longitude: xy.first()?.as_f64()?,
                latitude: xy.get(1)?.as_f64()?,
                speed: (!matches!(flag, Some(1 | 9)))
                    .then(|| number(p, "speed", 0.0, 100.0))
                    .flatten(),
                direction: (!matches!(flag, Some(2 | 9)))
                    .then(|| number(p, "dir", 0.0, 360.0))
                    .flatten(),
                gust: (!matches!(flag, Some(5 | 9)))
                    .then(|| number(p, "gust", 0.0, 130.0))
                    .flatten(),
                flag,
            })
        })
        .collect()
}

fn choose_wind(stations: Vec<Wind>, config: &Config) -> Result<Wind, String> {
    if let Some(name) = &config.wind_station {
        return stations
            .into_iter()
            .filter(|w| &w.station == name)
            .max_by_key(|w| w.time)
            .ok_or_else(|| format!("No wind observation for {name}"));
    }
    stations
        .into_iter()
        .filter(|w| !stale(w.time, 60) && w.speed.is_some() && w.direction.is_some())
        .filter(|w| distance_km(config.spot.coordinates(), (w.latitude, w.longitude)) <= 75.0)
        .min_by(|a, b| {
            distance_km(config.spot.coordinates(), (a.latitude, a.longitude)).total_cmp(
                &distance_km(config.spot.coordinates(), (b.latitude, b.longitude)),
            )
        })
        .ok_or_else(|| "No fresh wind station within 75 km. Choose a station in Settings.".into())
}

fn wind_history(value: &Value, station: &str) -> Result<Vec<Wind>, String> {
    let mut history: Vec<_> = parse_wind(value)
        .into_iter()
        .filter(|w| w.station == station)
        .collect();
    history.sort_by_key(|w| w.time);
    history.dedup_by_key(|w| w.time);
    if history.is_empty() {
        Err(format!("No wind history returned for {station}"))
    } else {
        Ok(history)
    }
}

pub fn fetch(config: &Config) -> Snapshot {
    let (waves, wind) = std::thread::scope(|scope| {
        let waves = scope.spawn(|| {
            get("waves", Some(config.spot.name()), true).and_then(|v| parse_waves(&v, config.spot))
        });
        let wind = get("met", None, false)
            .and_then(|v| choose_wind(parse_wind(&v), config))
            .map(|current| {
                let history = get("met", Some(&current.station), true)
                    .and_then(|v| wind_history(&v, &current.station));
                (current, history)
            });
        (
            waves
                .join()
                .unwrap_or_else(|_| Err("The wave request failed".into())),
            wind,
        )
    });
    let mut snapshot = Snapshot::default();
    match waves {
        Ok(w) => snapshot.waves = w,
        Err(e) => snapshot.wave_error = Some(e),
    }
    match wind {
        Ok((w, history)) => {
            snapshot.wind = Some(w);
            match history {
                Ok(history) => snapshot.wind_history = history,
                Err(error) => snapshot.wind_history_error = Some(error),
            }
        }
        Err(e) => snapshot.wind_error = Some(e),
    }
    snapshot
}

pub fn demo(spot: Spot) -> Snapshot {
    let now = Utc::now();
    let waves = (0..25)
        .map(|i| Wave {
            time: now - chrono::Duration::minutes((24 - i) * 30 + 12),
            height: Some(
                if spot == Spot::Perranporth { 1.9 } else { 1.1 }
                    + (i as f64 * 0.3).sin() * 0.25
                    + i as f64 * 0.015,
            ),
            period: Some(13.3),
            direction: Some(282.7),
            temperature: Some(16.9),
            flag: None,
        })
        .collect();
    let wind_history: Vec<_> = (0..=72)
        .map(|i| {
            let speed = 4.1 + (i as f64 * 0.12).sin() * 1.2 + (i as f64 * 0.4).cos() * 0.3;
            Wind {
                time: now - chrono::Duration::minutes((72 - i) * 10 + 7),
                station: "Perranporth".into(),
                latitude: 50.34611,
                longitude: -5.16177,
                speed: Some(speed),
                direction: Some(145.3 + (i as f64 * 0.09).sin() * 24.0),
                gust: Some(speed + 1.4),
                flag: None,
            }
        })
        .collect();
    Snapshot {
        waves,
        wind: wind_history.last().cloned(),
        wind_history,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wind_history_keeps_one_station_and_preserves_missing_samples() {
        let feature = |station: &str, date: &str, speed: Value, flag: Value| {
            json!({
                "properties": {"sensor": station,"date":date,"speed":speed,"dir":"30.1","gust":"12.2","QCflag":flag},
                "geometry":{"coordinates":[-5.16177,50.34611]}
            })
        };
        let payload = json!({"features":[
            feature("Perranporth","20261016#224000",json!("9.37"),Value::Null),
            feature("Penzance","20261016#224000",json!("4.1"),Value::Null),
            feature("Perranporth","20261016#223000",json!("8.1"),json!("1")),
            feature("Perranporth","20261016#222000",Value::Null,Value::Null),
            feature("Perranporth","20261016#224000",json!("9.37"),Value::Null)
        ]});
        let history = wind_history(&payload, "Perranporth").unwrap();
        assert_eq!(history.len(), 3);
        assert!(history.iter().all(|w| w.station == "Perranporth"));
        assert!(history[0].speed.is_none() && history[1].speed.is_none());
        assert_eq!(history[2].speed, Some(9.37));
        assert_eq!(history[2].direction, Some(30.1));
        assert!(wind_history(&payload, "Looe Bay").is_err());
    }

    #[test]
    fn buoy_payload_uses_peak_fields_and_sorts_observations() {
        let payload = json!({"features": [
            {"properties": {"sensor":"Perranporth", "date":"20261016#213000", "hs":"2.090", "tp":"13.330", "tz":"5.4050", "pdir":"282.700", "value":"999", "QCflag":null}},
            {"properties": {"sensor":"Porthleven", "date":"20261016#213000", "hs":"1.080"}},
            {"properties": {"sensor":"Perranporth", "date":"20261016#210000", "hs":"2.120", "tp":"12.5", "pdir":"0", "QCflag":"0"}}
        ]});
        let waves = parse_waves(&payload, Spot::Perranporth).unwrap();
        assert_eq!(waves.len(), 2);
        assert_eq!(waves[0].direction, Some(0.0));
        let latest = waves.last().unwrap();
        assert_eq!(latest.height, Some(2.09));
        assert_eq!(latest.period, Some(13.33));
        assert_eq!(latest.direction, Some(282.7));
        assert_eq!(latest.time.to_rfc3339(), "2026-10-16T21:30:00+00:00");
    }

    #[test]
    fn missing_sentinel_and_qc_values_never_become_plausible_readings() {
        let payload = json!({"features": [
            {"properties":{"sensor":"Perranporth", "date":"20261016#210000", "hs":"-99", "tp":null, "pdir":"999"}},
            {"properties":{"sensor":"Perranporth", "date":"20261016#213000", "hs":"2.1", "tp":"13.3", "pdir":"282.7", "QCflag":"2"}}
        ]});
        let waves = parse_waves(&payload, Spot::Perranporth).unwrap();
        assert!(
            waves[0].height.is_none() && waves[0].period.is_none() && waves[0].direction.is_none()
        );
        assert_eq!(waves[1].height, Some(2.1));
        assert!(waves[1].period.is_none() && waves[1].direction.is_none());
    }

    fn station(name: &str, age: i64, lat: f64, lon: f64, speed: Option<f64>) -> Wind {
        Wind {
            time: Utc::now() - chrono::Duration::minutes(age),
            station: name.into(),
            latitude: lat,
            longitude: lon,
            speed,
            direction: Some(0.0),
            gust: None,
            flag: None,
        }
    }

    #[test]
    fn automatic_wind_skips_nearer_stale_or_missing_data_and_rejects_distant_stations() {
        let config = Config {
            spot: Spot::Porthleven,
            ..Default::default()
        };
        let chosen = choose_wind(
            vec![
                station("Penzance", 120, 50.117, -5.530, Some(3.0)),
                station("Missing", 5, 50.062, -5.307, None),
                station("Perranporth", 5, 50.346, -5.162, Some(4.0)),
                station("Looe Bay", 5, 50.350, -4.453, Some(2.0)),
            ],
            &config,
        )
        .unwrap();
        assert_eq!(chosen.station, "Perranporth");
        assert!(
            choose_wind(
                vec![station("Brighton", 2, 50.812, -0.101, Some(4.0))],
                &config
            )
            .is_err()
        );
    }

    #[test]
    fn manual_wind_preserves_selected_source_even_when_delayed() {
        let config = Config {
            wind_station: Some("Penzance".into()),
            ..Default::default()
        };
        let chosen = choose_wind(
            vec![
                station("Penzance", 120, 50.117, -5.530, Some(3.0)),
                station("Perranporth", 5, 50.346, -5.162, Some(4.0)),
            ],
            &config,
        )
        .unwrap();
        assert_eq!(chosen.station, "Penzance");
        assert!(stale(chosen.time, 60));
    }
}
