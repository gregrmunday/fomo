//! Fomo forecast file reader. Uses the app's existing chrono/serde dependencies.
//! Direction is a magnetic FROM bearing, consistent with the CCO observations.
use chrono::{DateTime, Utc};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, path::Path, time::SystemTime};

#[derive(Clone, Debug, Deserialize)]
pub struct ForecastBundle {
    pub schema: String,
    pub schema_version: u32,
    pub model_version: String,
    pub issued_at_utc: DateTime<Utc>,
    pub generated_at_utc: DateTime<Utc>,
    pub refresh_after_utc: DateTime<Utc>,
    pub stale_after_utc: DateTime<Utc>,
    pub units: Units,
    pub warnings: Vec<String>,
    pub forecasts: Vec<BuoyForecast>,
    #[serde(default)]
    pub swell_panel: Option<SwellPanel>,
    // When the file was written, so callers can skip re-reading an unchanged forecast.
    #[serde(skip)]
    pub modified: Option<SystemTime>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct SwellPanel {
    pub window_start_utc: DateTime<Utc>,
    pub window_end_utc: DateTime<Utc>,
    pub warning: Option<String>,
    pub forecasts: Vec<SwellSeries>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct SwellSeries {
    pub station_id: String,
    pub points: Vec<SwellPoint>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct SwellPoint {
    pub valid_at_utc: DateTime<Utc>,
    pub hs_m: Quantiles,
    pub tp_s: Quantiles,
    pub direction: Direction,
    pub issued_at_utc: DateTime<Utc>,
    pub generated_at_utc: DateTime<Utc>,
    pub model_version: String,
    pub origin: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Units {
    pub height: String,
    pub period: String,
    pub direction: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct BuoyForecast {
    pub station_id: String,
    pub name: String,
    pub status: String,
    pub supported_through_utc: DateTime<Utc>,
    pub points: Vec<ForecastPoint>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct ForecastPoint {
    pub valid_at_utc: DateTime<Utc>,
    pub lead_hours: f64,
    pub hs_m: Quantiles,
    pub tp_s: Quantiles,
    pub direction: Direction,
    pub expert_weights: ExpertWeights,
}
#[derive(Clone, Debug, Deserialize)]
pub struct ExpertWeights {
    pub height: BTreeMap<String, f64>,
    pub period: BTreeMap<String, f64>,
    pub direction: BTreeMap<String, f64>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Quantiles {
    pub p10: f64,
    pub p50: f64,
    pub p90: f64,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Direction {
    pub from_deg: f64,
    pub reference: String,
    pub parameter: String,
    pub interval80: CircularArc,
    pub resultant_length: f64,
}
#[derive(Clone, Debug, Deserialize)]
pub struct CircularArc {
    pub centre_deg: f64,
    pub half_width_deg: f64,
    pub start_deg: f64,
    pub end_deg: f64,
    pub wraps_north: bool,
    pub full_circle: bool,
}
impl Quantiles {
    fn valid(&self) -> bool {
        [self.p10, self.p50, self.p90]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0)
            && self.p10 <= self.p50
            && self.p50 <= self.p90
    }
}
impl ForecastBundle {
    pub fn load(path: &Path) -> Result<Self, String> {
        let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
        if metadata.len() > 4 * 1024 * 1024 {
            return Err("Forecast file too large".into());
        }
        let mut bundle: Self = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("Unreadable forecast: {e}"))?;
        bundle.validate()?;
        bundle.modified = metadata.modified().ok();
        Ok(bundle)
    }
    pub fn is_stale(&self) -> bool {
        Utc::now() > self.stale_after_utc
    }
    pub fn for_station(&self, id: &str) -> Option<&BuoyForecast> {
        self.forecasts.iter().find(|f| f.station_id == id)
    }
    // Model values for the dashboard's swell chart, ordered by valid time. Panel
    // predictions cover the past; the current forecast replaces them from its
    // generation time onwards.
    pub fn predictions_for(&self, id: &str) -> Vec<SwellPoint> {
        let mut predictions = BTreeMap::new();
        let panel = self
            .swell_panel
            .iter()
            .flat_map(|p| &p.forecasts)
            .find(|s| s.station_id == id);
        for point in panel.into_iter().flat_map(|s| &s.points) {
            predictions.insert(point.valid_at_utc, point.clone());
        }
        let current = self.for_station(id).into_iter().flat_map(|b| &b.points);
        for point in current.filter(|p| p.valid_at_utc >= self.generated_at_utc) {
            predictions.insert(
                point.valid_at_utc,
                SwellPoint {
                    valid_at_utc: point.valid_at_utc,
                    hs_m: point.hs_m.clone(),
                    tp_s: point.tp_s.clone(),
                    direction: point.direction.clone(),
                    issued_at_utc: self.issued_at_utc,
                    generated_at_utc: self.generated_at_utc,
                    model_version: self.model_version.clone(),
                    origin: "forecast".into(),
                },
            );
        }
        predictions.into_values().collect()
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != "fomo.wave_forecast" || self.schema_version != 2 {
            return Err("Unsupported forecast schema".into());
        }
        if self.units.height != "m" || self.units.period != "s" || self.units.direction != "degree"
        {
            return Err("Unexpected forecast units".into());
        }
        if self.issued_at_utc > self.generated_at_utc
            || self.stale_after_utc < self.issued_at_utc
            || self.refresh_after_utc < self.issued_at_utc
        {
            return Err("Invalid forecast issue time".into());
        }
        if self.forecasts.len() != 2
            || self.for_station("76").is_none()
            || self.for_station("107").is_none()
        {
            return Err("Missing or duplicate buoy".into());
        }
        if let Some(panel) = &self.swell_panel {
            if panel.window_end_utc - panel.window_start_utc != chrono::Duration::hours(24)
                || panel.forecasts.len() != 2
                || panel.forecasts[0].station_id == panel.forecasts[1].station_id
            {
                return Err("Invalid 24-hour swell panel".into());
            }
            for series in &panel.forecasts {
                if !["76", "107"].contains(&series.station_id.as_str()) {
                    return Err("Unknown panel buoy".into());
                }
                let mut previous = None;
                for point in &series.points {
                    if point.valid_at_utc < panel.window_start_utc
                        || point.valid_at_utc > panel.window_end_utc
                        || !point.hs_m.valid()
                        || !point.tp_s.valid()
                        || point.issued_at_utc >= point.valid_at_utc
                        || !["forecast", "archived_forecast", "reconstructed"]
                            .contains(&point.origin.as_str())
                        || previous.is_some_and(|t| t >= point.valid_at_utc)
                        || (point.origin == "archived_forecast"
                            && point.generated_at_utc > point.valid_at_utc)
                    {
                        return Err("Invalid swell-panel prediction".into());
                    }
                    previous = Some(point.valid_at_utc);
                }
            }
        }
        for buoy in &self.forecasts {
            if buoy.points.len() != 168 {
                return Err("Expected 168 hourly points".into());
            }
            let mut previous = None;
            for p in &buoy.points {
                if p.valid_at_utc <= self.issued_at_utc
                    || !p.hs_m.valid()
                    || !p.tp_s.valid()
                    || !p.lead_hours.is_finite()
                    || !(0.0..=168.0).contains(&p.lead_hours)
                {
                    return Err("Invalid forecast values or time".into());
                }
                if previous
                    .is_some_and(|old: DateTime<Utc>| (p.valid_at_utc - old).num_seconds() != 3600)
                {
                    return Err("Forecast hours are not consecutive".into());
                }
                previous = Some(p.valid_at_utc);
                let d = &p.direction;
                if !d.from_deg.is_finite()
                    || !(0.0..360.0).contains(&d.from_deg)
                    || d.reference != "magnetic"
                    || d.parameter != "spectral_peak"
                    || !d.interval80.half_width_deg.is_finite()
                    || !(0.0..=180.0).contains(&d.interval80.half_width_deg)
                    || !d.resultant_length.is_finite()
                    || !(0.0..=1.0).contains(&d.resultant_length)
                {
                    return Err("Invalid circular direction".into());
                }
                let arc = &d.interval80;
                if !arc.centre_deg.is_finite()
                    || !(0.0..360.0).contains(&arc.centre_deg)
                    || !arc.start_deg.is_finite()
                    || !(0.0..360.0).contains(&arc.start_deg)
                    || !arc.end_deg.is_finite()
                    || !(0.0..=360.0).contains(&arc.end_deg)
                    || arc.full_circle != (arc.half_width_deg >= 180.0)
                {
                    return Err("Invalid direction uncertainty arc".into());
                }
                for weights in [
                    &p.expert_weights.height,
                    &p.expert_weights.period,
                    &p.expert_weights.direction,
                ] {
                    if weights.len() != 2
                        || !["ecmwf_wam", "ncep_gfswave025"]
                            .iter()
                            .all(|k| weights.contains_key(*k))
                    {
                        return Err("Unexpected expert identities".into());
                    }
                    if weights
                        .values()
                        .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
                        || (weights.values().sum::<f64>() - 1.0).abs() > 0.0001
                    {
                        return Err("Invalid expert allocation".into());
                    }
                }
            }
            if previous != Some(buoy.supported_through_utc) {
                return Err("Forecast horizon mismatch".into());
            }
        }
        Ok(())
    }
}

pub fn project_dir() -> std::path::PathBuf {
    std::env::var_os("FOMO_FORECAST_PROJECT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
                .join("Documents/BuoyData/deepwater-models")
        })
}
pub fn output_path() -> std::path::PathBuf {
    std::env::var_os("FOMO_FORECAST_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| project_dir().join("forecasts/fomo-v2.json"))
}
// None when no forecast has been generated yet.
pub fn load_from(path: &Path) -> Result<Option<ForecastBundle>, String> {
    if !path.exists() {
        return Ok(None);
    }
    ForecastBundle::load(path).map(Some)
}
pub fn load_local() -> Result<Option<ForecastBundle>, String> {
    load_from(&output_path())
}
pub fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

// The worker's final message, read from the end of its log. Decoding is lossy so that
// stray bytes, or a character split where the tail starts, cannot hide the error.
fn last_log_line(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    const TAIL: u64 = 64 * 1024;
    let mut file = fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(TAIL)))
        .ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    String::from_utf8_lossy(&bytes)
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(250).collect())
}

pub fn regenerate() -> Result<ForecastBundle, String> {
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    let project = project_dir();
    let script = project.join("scripts/deepwater");
    if !script.is_file() {
        return Err("Forecast worker missing. Set FOMO_FORECAST_PROJECT.".into());
    }
    let log_path = project.join("data/fomo-refresh.log");
    std::fs::create_dir_all(project.join("data")).map_err(|e| e.to_string())?;
    let log = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;
    let mut child = Command::new("/bin/sh")
        .arg(script)
        .args(["forecast-fomo", "--device", "mps", "--output"])
        .arg(output_path())
        .current_dir(&project)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log))
        .spawn()
        .map_err(|e| format!("Cannot start forecast worker: {e}"))?;
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            if status.success() {
                return load_local()?.ok_or_else(|| "The forecast worker wrote no file".into());
            }
            return Err(last_log_line(&log_path).unwrap_or_else(|| "Forecast worker failed".into()));
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("Forecast refresh timed out. The previous forecast is retained.".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn at(text: &str) -> DateTime<Utc> {
        text.parse().unwrap()
    }

    // A valid bundle: issued 09:00, generated 09:05, 168 hourly points per buoy from
    // 10:00, and a 24-hour swell panel from 21:00 the previous evening.
    pub(crate) fn sample() -> Value {
        let issued = at("2026-10-19T09:00:00Z");
        let generated = at("2026-10-19T09:05:00Z");
        let weights = json!({"ecmwf_wam": 0.6, "ncep_gfswave025": 0.4});
        let direction = json!({
            "from_deg": 285.0, "reference": "magnetic", "parameter": "spectral_peak",
            "interval80": {"centre_deg": 285.0, "half_width_deg": 20.0, "start_deg": 265.0,
                           "end_deg": 305.0, "wraps_north": false, "full_circle": false},
            "resultant_length": 0.9
        });
        let quantiles =
            |median: f64| json!({"p10": median * 0.8, "p50": median, "p90": median * 1.2});
        let buoy = |id: &str, name: &str| {
            let points: Vec<_> = (1..=168)
                .map(|lead| {
                    json!({
                        "valid_at_utc": issued + chrono::Duration::hours(lead),
                        "lead_hours": lead as f64,
                        "hs_m": quantiles(1.5),
                        "tp_s": quantiles(12.5),
                        "direction": direction,
                        "expert_weights": {"height": weights, "period": weights, "direction": weights}
                    })
                })
                .collect();
            json!({"station_id": id, "name": name, "status": "experimental",
                   "supported_through_utc": issued + chrono::Duration::hours(168), "points": points})
        };
        let start = at("2026-10-18T21:00:00Z");
        let panel = |id: &str| {
            let points: Vec<_> = (0..=24)
                .map(|hour| {
                    let valid = start + chrono::Duration::hours(hour);
                    let issued = valid - chrono::Duration::hours(6);
                    json!({
                        "valid_at_utc": valid, "hs_m": quantiles(1.2), "tp_s": quantiles(11.0),
                        "direction": direction, "issued_at_utc": issued,
                        "generated_at_utc": issued + chrono::Duration::minutes(5),
                        "model_version": "panel",
                        "origin": if valid < generated { "reconstructed" } else { "forecast" }
                    })
                })
                .collect();
            json!({"station_id": id, "points": points})
        };
        json!({
            "schema": "fomo.wave_forecast", "schema_version": 2, "model_version": "test",
            "issued_at_utc": issued, "generated_at_utc": generated,
            "refresh_after_utc": at("2026-10-19T15:00:00Z"),
            "stale_after_utc": at("2026-10-19T15:00:00Z"),
            "units": {"height": "m", "period": "s", "direction": "degree"},
            "warnings": [],
            "forecasts": [buoy("76", "Perranporth"), buoy("107", "Porthleven")],
            "swell_panel": {
                "window_start_utc": start, "window_end_utc": start + chrono::Duration::hours(24),
                "warning": null, "forecasts": [panel("76"), panel("107")]
            }
        })
    }

    type Mutation = fn(&mut Value);

    fn check(mutate: Mutation) -> Result<(), String> {
        let mut value = sample();
        mutate(&mut value);
        serde_json::from_value::<ForecastBundle>(value)
            .map_err(|e| e.to_string())?
            .validate()
    }

    #[test]
    fn sample_forecast_is_valid_with_or_without_a_swell_panel() {
        assert_eq!(check(|_| {}), Ok(()));
        assert_eq!(check(|v| v["swell_panel"] = Value::Null), Ok(()));
    }

    #[test]
    fn validation_rejects_each_broken_rule() {
        let cases: [(&str, Mutation); 14] = [
            ("Unsupported forecast schema", |v| {
                v["schema_version"] = json!(3)
            }),
            ("Unexpected forecast units", |v| {
                v["units"]["height"] = json!("ft")
            }),
            ("Invalid forecast issue time", |v| {
                v["generated_at_utc"] = json!("2026-10-19T08:00:00Z")
            }),
            ("Missing or duplicate buoy", |v| {
                v["forecasts"][1]["station_id"] = json!("76")
            }),
            ("Invalid 24-hour swell panel", |v| {
                v["swell_panel"]["forecasts"][1]["station_id"] = json!("76")
            }),
            ("Invalid swell-panel prediction", |v| {
                v["swell_panel"]["forecasts"][0]["points"][3]["origin"] = json!("guess")
            }),
            ("Expected 168 hourly points", |v| {
                v["forecasts"][0]["points"].as_array_mut().unwrap().pop();
            }),
            ("Forecast hours are not consecutive", |v| {
                v["forecasts"][0]["points"][5]["valid_at_utc"] = json!("2026-10-19T15:30:00Z")
            }),
            ("Invalid forecast values or time", |v| {
                v["forecasts"][0]["points"][0]["hs_m"]["p10"] = json!(9.0)
            }),
            ("Invalid circular direction", |v| {
                v["forecasts"][0]["points"][0]["direction"]["from_deg"] = json!(360.0)
            }),
            ("Invalid direction uncertainty arc", |v| {
                v["forecasts"][0]["points"][0]["direction"]["interval80"]["full_circle"] =
                    json!(true)
            }),
            ("Unexpected expert identities", |v| {
                v["forecasts"][0]["points"][0]["expert_weights"]["height"] =
                    json!({"ecmwf_wam": 0.5, "dwd_ewam": 0.5})
            }),
            ("Invalid expert allocation", |v| {
                v["forecasts"][0]["points"][0]["expert_weights"]["period"] =
                    json!({"ecmwf_wam": 0.7, "ncep_gfswave025": 0.4})
            }),
            ("Forecast horizon mismatch", |v| {
                v["forecasts"][0]["supported_through_utc"] = json!("2026-10-26T08:00:00Z")
            }),
        ];
        for (expected, mutate) in cases {
            assert_eq!(check(mutate), Err(expected.into()));
        }
    }

    #[test]
    fn chart_predictions_switch_to_the_current_forecast_at_generation_time() {
        let bundle: ForecastBundle = serde_json::from_value(sample()).unwrap();
        let predictions = bundle.predictions_for("76");
        // 25 panel hours and 168 forecast hours, overlapping from 10:00 to 21:00.
        assert_eq!(predictions.len(), 25 + 168 - 12);
        assert!(
            predictions
                .windows(2)
                .all(|w| w[0].valid_at_utc < w[1].valid_at_utc)
        );
        for p in &predictions {
            if p.valid_at_utc >= bundle.generated_at_utc {
                assert_eq!(
                    (p.origin.as_str(), p.issued_at_utc),
                    ("forecast", bundle.issued_at_utc)
                );
            } else {
                assert_eq!(p.model_version, "panel");
                assert_eq!(p.issued_at_utc, p.valid_at_utc - chrono::Duration::hours(6));
            }
        }
        assert!(bundle.predictions_for("999").is_empty());
    }

    #[test]
    fn worker_error_comes_from_the_end_of_a_long_log() {
        let path = std::env::temp_dir().join(format!("fomo-log-test-{}.log", std::process::id()));
        // Over 64 KB of three-byte characters, then an invalid byte, then the message
        // that matters. A 64 KB window at either end splits a character here; reading
        // the head as strict UTF-8 used to yield only the generic failure message.
        let mut log = "…".repeat(30_000).into_bytes();
        log.extend_from_slice(b"stray \xff byte\nRuntimeError: MPS backend out of memory\n\n");
        assert_eq!(
            log[log.len() - 64 * 1024] & 0xC0,
            0x80,
            "tail must start mid-character"
        );
        fs::write(&path, &log).unwrap();
        let line = last_log_line(&path);
        fs::remove_file(&path).unwrap();
        assert!(log.len() > 64 * 1024);
        assert_eq!(
            line.as_deref(),
            Some("RuntimeError: MPS backend out of memory")
        );
        assert_eq!(last_log_line(&path), None);
    }
}
