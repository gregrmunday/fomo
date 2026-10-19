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
pub fn load_local() -> Result<Option<ForecastBundle>, String> {
    let path = output_path();
    if !path.exists() {
        return Ok(None);
    }
    ForecastBundle::load(&path).map(Some)
}
pub fn modified() -> Option<SystemTime> {
    fs::metadata(output_path()).and_then(|m| m.modified()).ok()
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
mod tests {
    use super::*;

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
