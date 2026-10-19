mod data;
mod model;
mod forecast;
#[cfg(target_os = "macos")]
mod forecast_ui;

#[cfg(target_os = "macos")]
mod ui;

fn main() {
    if std::env::args().any(|a| a == "--refresh-forecast") {
        match forecast::regenerate() {
            Ok(bundle) => println!("GPU forecast refreshed: {} · {} buoys", bundle.model_version, bundle.forecasts.len()),
            Err(error) => { eprintln!("{error}"); std::process::exit(1); }
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
