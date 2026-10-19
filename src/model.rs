use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Spot {
    #[default]
    Perranporth,
    Porthleven,
}

impl Spot {
    pub fn name(self) -> &'static str {
        match self {
            Self::Perranporth => "Perranporth",
            Self::Porthleven => "Porthleven",
        }
    }
    pub fn short(self) -> &'static str {
        match self {
            Self::Perranporth => "PR",
            Self::Porthleven => "PL",
        }
    }
    pub fn id(self) -> &'static str {
        match self {
            Self::Perranporth => "76",
            Self::Porthleven => "107",
        }
    }
    pub fn coordinates(self) -> (f64, f64) {
        match self {
            Self::Perranporth => (50.35323, -5.17496),
            Self::Porthleven => (50.06257, -5.30790),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WaveUnit {
    Metres,
    #[default]
    Feet,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindUnit {
    Knots,
    #[default]
    Mph,
    Kmh,
    MetresPerSecond,
}

impl WindUnit {
    pub fn label(self) -> &'static str {
        match self {
            Self::Knots => "kn",
            Self::Mph => "mph",
            Self::Kmh => "km/h",
            Self::MetresPerSecond => "m/s",
        }
    }
    pub fn convert(self, ms: f64) -> f64 {
        ms * match self {
            Self::Knots => 1.943_844,
            Self::Mph => 2.236_936,
            Self::Kmh => 3.6,
            Self::MetresPerSecond => 1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub spot: Spot,
    pub wave_unit: WaveUnit,
    pub wind_unit: WindUnit,
    pub refresh_minutes: u32,
    // None selects the closest fresh, usable observation within 75 km.
    pub wind_station: Option<String>,
    pub compact: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            spot: Spot::default(),
            wave_unit: WaveUnit::default(),
            wind_unit: WindUnit::default(),
            refresh_minutes: 5,
            wind_station: None,
            compact: false,
        }
    }
}

fn support_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/Fomo"))
}

impl Config {
    pub fn load() -> Self {
        let mut config: Self = support_dir()
            .and_then(|p| fs::read(p.join("settings.json")).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        if ![5, 10, 15, 30].contains(&config.refresh_minutes) {
            config.refresh_minutes = 5;
        }
        config
    }
    pub fn save(&self) -> Result<(), String> {
        save_json("settings.json", self)
    }
    pub fn height(&self, metres: f64) -> f64 {
        metres
            * if self.wave_unit == WaveUnit::Feet {
                3.280_84
            } else {
                1.0
            }
    }
    pub fn height_unit(&self) -> &'static str {
        if self.wave_unit == WaveUnit::Feet {
            "ft"
        } else {
            "m"
        }
    }
    pub fn height_text(&self, value: Option<f64>) -> String {
        value
            .map(|v| format!("{:.1}{}", self.height(v), self.height_unit()))
            .unwrap_or_else(|| format!("—{}", self.height_unit()))
    }
    pub fn wind_text(&self, value: Option<f64>) -> String {
        value
            .map(|v| format!("{:.0}{}", self.wind_unit.convert(v), self.wind_unit.label()))
            .unwrap_or_else(|| format!("—{}", self.wind_unit.label()))
    }
}

pub fn save_json(name: &str, value: &impl Serialize) -> Result<(), String> {
    let dir = support_dir().ok_or("Cannot locate your home folder")?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let temporary = dir.join(format!("{name}.tmp"));
    fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
    fs::rename(temporary, dir.join(name)).map_err(|e| e.to_string())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wave {
    pub time: DateTime<Utc>,
    pub height: Option<f64>,
    pub period: Option<f64>,
    pub direction: Option<f64>,
    pub temperature: Option<f64>,
    pub flag: Option<u32>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wind {
    pub time: DateTime<Utc>,
    pub station: String,
    pub latitude: f64,
    pub longitude: f64,
    pub speed: Option<f64>,
    pub direction: Option<f64>,
    pub gust: Option<f64>,
    pub flag: Option<u32>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub waves: Vec<Wave>,
    pub wind: Option<Wind>,
    #[serde(default)]
    pub wind_history: Vec<Wind>,
    #[serde(default)]
    pub wind_history_error: Option<String>,
    pub wave_error: Option<String>,
    pub wind_error: Option<String>,
}

impl Snapshot {
    pub fn latest(&self) -> Option<&Wave> {
        self.waves.last()
    }
    pub fn load(spot: Spot) -> Self {
        support_dir()
            .and_then(|p| fs::read(p.join(format!("cache-{}.json", spot.id()))).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, spot: Spot) -> Result<(), String> {
        save_json(&format!("cache-{}.json", spot.id()), self)
    }
}

pub fn age_minutes(time: DateTime<Utc>) -> i64 {
    (Utc::now() - time).num_minutes().max(0)
}
pub fn stale(time: DateTime<Utc>, minutes: i64) -> bool {
    age_minutes(time) > minutes || time > Utc::now() + chrono::Duration::minutes(5)
}
// Wall-clock time, because Instant stops while a Mac sleeps.
pub fn fetch_due(last_fetch: Option<DateTime<Utc>>, refresh_minutes: u32) -> bool {
    last_fetch.is_none_or(|t| Utc::now() - t >= chrono::Duration::minutes(refresh_minutes.into()))
}
pub fn age_text(time: DateTime<Utc>) -> String {
    let m = age_minutes(time);
    if m == 0 {
        "just now".into()
    } else if m < 60 {
        format!("{m}m ago")
    } else if m < 1440 {
        format!("{}h {}m ago", m / 60, m % 60)
    } else {
        format!("{}d ago", m / 1440)
    }
}

pub fn compass(degrees: f64) -> &'static str {
    const POINTS: [&str; 16] = [
        "N", "NNE", "NE", "ENE", "E", "ESE", "SE", "SSE", "S", "SSW", "SW", "WSW", "W", "WNW",
        "NW", "NNW",
    ];
    POINTS[((degrees.rem_euclid(360.0) / 22.5).round() as usize) % 16]
}

// A direction is where the sea/wind comes FROM; vectors point in its travel direction.
// AppKit coordinates increase upwards. Preserve the original fractional bearing.
pub fn travel_vector(degrees: f64) -> (f64, f64) {
    let radians = (degrees + 180.0).to_radians();
    (radians.sin(), radians.cos())
}
pub fn direction_text(value: Option<f64>) -> String {
    value
        .map(|d| format!("{:.1}° {}", d.rem_euclid(360.0), compass(d)))
        .unwrap_or_else(|| "—".into())
}

pub fn distance_km(a: (f64, f64), b: (f64, f64)) -> f64 {
    let dlat = (b.0 - a.0).to_radians();
    let dlon = (b.1 - a.1).to_radians();
    let h = (dlat / 2.0).sin().powi(2)
        + a.0.to_radians().cos() * b.0.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    6371.0 * 2.0 * h.sqrt().min(1.0).asin()
}

pub fn menu_title(config: &Config, data: &Snapshot, loading: bool) -> String {
    let wave = data.latest();
    let wind = data.wind.as_ref();
    let wave_old = wave.is_some_and(|w| {
        stale(w.time, 90) || w.height.is_none() || w.period.is_none() || w.direction.is_none()
    }) || data.wave_error.is_some();
    let wind_old = wind
        .is_some_and(|w| stale(w.time, 60) || w.speed.is_none() || w.direction.is_none())
        || data.wind_error.is_some();
    let marker = if wave_old || wind_old {
        " !"
    } else if loading {
        " ·"
    } else {
        ""
    };
    let h = config.height_text(wave.and_then(|w| w.height));
    let p = wave
        .and_then(|w| w.period)
        .map(|p| format!("{p:.0}s"))
        .unwrap_or_else(|| "—s".into());
    let d = |v: Option<f64>| {
        v.map(|d| format!("{:.1}°{}", d.rem_euclid(360.0), compass(d)))
            .unwrap_or_else(|| "—".into())
    };
    let speed = config.wind_text(wind.and_then(|w| w.speed));
    if config.compact {
        format!("{} {h} {p} · {speed}{marker}", config.spot.short())
    } else {
        format!(
            "{} {h} {p} {} · {speed} {}{marker}",
            config.spot.short(),
            d(wave.and_then(|w| w.direction)),
            d(wind.and_then(|w| w.direction))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_without_wind_history_remains_readable() {
        let old_cache = r#"{"waves":[],"wind":null,"wave_error":null,"wind_error":null}"#;
        let snapshot: Snapshot = serde_json::from_str(old_cache).unwrap();
        assert!(snapshot.wind_history.is_empty());
        assert!(snapshot.wind_history_error.is_none());
    }

    #[test]
    fn directions_distinguish_origin_from_travel_and_wrap_north() {
        assert_eq!(compass(360.0), "N");
        assert_eq!(compass(282.7), "WNW");
        assert!((travel_vector(270.0).0 - 1.0).abs() < 1e-12);
        assert!((travel_vector(0.0).1 + 1.0).abs() < 1e-12);
        assert!((travel_vector(30.0).0 + 0.5).abs() < 1e-12);
        assert!((travel_vector(30.0).1 + 3.0_f64.sqrt() / 2.0).abs() < 1e-12);
        assert_ne!(travel_vector(30.0), travel_vector(30.1));
        assert_eq!(direction_text(Some(360.0)), "0.0° N");
    }

    #[test]
    fn unit_conversions_preserve_calm_and_missing_values() {
        let config = Config {
            wave_unit: WaveUnit::Feet,
            wind_unit: WindUnit::Knots,
            ..Default::default()
        };
        assert_eq!(config.height_text(Some(1.0)), "3.3ft");
        assert_eq!(config.wind_text(Some(10.0)), "19kn");
        assert_eq!(config.wind_text(Some(0.0)), "0kn");
        assert_eq!(config.wind_text(None), "—kn");
        assert_eq!(
            distance_km(
                Spot::Porthleven.coordinates(),
                Spot::Perranporth.coordinates()
            )
            .round(),
            34.0
        );
    }

    #[test]
    fn fetch_is_due_once_the_refresh_interval_has_passed() {
        let ago = |minutes| Some(Utc::now() - chrono::Duration::minutes(minutes));
        assert!(fetch_due(None, 5));
        assert!(!fetch_due(ago(4), 5));
        assert!(fetch_due(ago(5), 5));
        assert!(fetch_due(ago(8 * 60), 30));
    }

    #[test]
    fn menu_marks_independent_stale_wind_and_network_failure() {
        let mut snapshot = crate::data::demo(Spot::Perranporth);
        assert!(!menu_title(&Config::default(), &snapshot, false).ends_with('!'));
        snapshot.wind.as_mut().unwrap().time = Utc::now() - chrono::Duration::hours(2);
        assert!(menu_title(&Config::default(), &snapshot, false).ends_with('!'));
        snapshot.wind.as_mut().unwrap().time = Utc::now();
        snapshot.wave_error = Some("Offline".into());
        assert!(menu_title(&Config::default(), &snapshot, false).ends_with('!'));
    }
}
