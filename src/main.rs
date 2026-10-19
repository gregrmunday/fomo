mod data;
mod forecast;
mod model;

#[cfg(target_os = "macos")]
mod ui;

fn main() {
    if std::env::args().any(|a| a == "--version") {
        println!("fomo {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if std::env::args().any(|a| a == "--refresh-forecast") {
        let how = if forecast::runs_locally() {
            "generated on this Mac's GPU"
        } else {
            "downloaded"
        };
        match forecast::refresh() {
            Ok(bundle) => println!(
                "Forecast {how}: {} · {} buoys",
                bundle.model_version,
                bundle.forecasts.len()
            ),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }

    if std::env::args().any(|a| a == "--check-data") {
        let config = model::Config::default();
        let result = data::fetch(&config);
        println!(
            "{}",
            serde_json::to_string_pretty(&result).expect("serialize observations")
        );
        if result.wave_error.is_some()
            || result.wind_error.is_some()
            || result.wind_history_error.is_some()
        {
            std::process::exit(1);
        }
        return;
    }
    #[cfg(target_os = "macos")]
    ui::run();
    #[cfg(not(target_os = "macos"))]
    eprintln!("Fomo is a macOS menu bar app. Build and run it on a Mac.");
}

#[cfg(test)]
mod tests {
    #[test]
    fn bundle_version_matches_the_crate() {
        let plist = include_str!("../packaging/Info.plist");
        let short = format!(
            "<key>CFBundleShortVersionString</key><string>{}</string>",
            env!("CARGO_PKG_VERSION")
        );
        assert!(plist.contains(&short), "update packaging/Info.plist");
    }
}
