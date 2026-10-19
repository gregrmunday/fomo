# Fomo

Cornwall's buoys, at a glance. A small **Rust macOS menu bar app** for Perranporth and Porthleven, built directly with AppKit. Native controls, system fonts, light and dark appearance, and no web view or JavaScript runtime.

```text
  PR → WNW    ↘ NNW
≋ 6.9ft 13s   20mph
```

Click the readings to open the conditions panel: significant wave height, peak period and direction, sea temperature (°C), a 12 hour wave history, wind speed, direction and gusts. Settings let you switch spots, units, wind stations, refresh interval, or use a shorter menu bar title. The wave icon remains in the menu bar while the app runs; no Dock window is needed.

The default menu bar layout stacks each direction above its readings, with a thin vertical divider between waves and wind. Arrows rotate continuously using the original fractional bearings. Hover over the wave history to see time and height, or the 12 hour wind history to see time, speed, gusts and direction. Wave height defaults to feet and wind speed (including gusts) to mph; both can be changed in Settings.

## Run

Requires macOS 12 or later, Rust with Cargo, and Apple's Command Line Tools (`xcode-select --install` if missing).

```sh
./scripts/build-app.sh
open dist/Fomo.app
```

The script builds for this Mac's architecture and signs the local app with an ad hoc signature. You can drag `dist/Fomo.app` into Applications. For automatic startup, add that app under System Settings → General → Login Items.

For development:

```sh
cargo run
cargo run -- --demo --show    # Sample observations, labelled DEMO; no requests or saved changes
cargo run -- --check-data     # Print real Perranporth observations and API errors; no GUI
```

With the panel open, **⌘R** refreshes the conditions or forecast page, **⌘,** opens settings and **⌘Q** quits from any page, and **⌘C** copies selected text. Escape or clicking elsewhere dismisses the panel. Hold **⌘** and drag the menu bar item to position it near the right edge. macOS controls menu bar placement and visibility; if you want it visible while using other apps, turn off automatic menu bar hiding in System Settings. The compact title helps on MacBooks with limited menu bar space around the notch.

## What the readings mean

**Height is combined significant wave height (Hs)** at the buoy. It is neither breaking surf height nor an isolated primary swell height. The real time API does not provide partitioned primary/secondary swell heights. **Period (Tp) and direction (pdir)** describe the most energetic part of the wave spectrum; that peak may switch between systems. See the provider's [Wave Parameter Handbook](https://coastalmonitoring.org/ccoresources/waveparameterhandbook/).

Bearings and compass labels say where the waves or wind come **from**. Arrows point in the direction of **travel**: a rightward arrow labelled `W` means coming from the west and travelling east. The arrows use the exact numerical bearing without snapping to compass sectors; compass labels still describe one of the 16 standard sectors. The provider documents wave direction relative to magnetic north. Native API units are metres, seconds, degrees and metres per second; conversions happen only for display.

**Wind comes from a shore weather station.** Perranporth has its own station; Porthleven has no station in this API. Automatic selection chooses the nearest station with both wind measurements less than 60 minutes old, within 75 km of the selected buoy. The panel always names the station and shows its distance and observation age. A station on another coast can have different conditions at your beach. Manual selection keeps the selected station, with delayed or missing values clearly indicated.

The app fetches every 5 minutes by default; the provider publishes discrete observations, not a continuous stream. It also fetches about ten seconds after the Mac wakes, and when you open the panel, whenever the last fetch is older than the refresh interval. Waves older than 90 minutes and wind older than 60 minutes are marked delayed. `!` in the menu bar means delayed data or a fetch error; hover or click for details. Each source retains its own timestamp. A missing or invalid value is `—`, never zero. Available provider QC flags suppress affected measurements. These real time records have not undergone full quality control.

Last wave observations are cached locally. On request failures the app retains the last reading and reports the error, then retries at the next interval. Settings and caches are in `~/Library/Application Support/Fomo/`. Demo mode never writes them.

## API

Uses the public development key published on the [official API page](https://coastalmonitoring.org/ccoresources/api/). Wave requests use `observations/waves/latest.geojson?sensor=…&duration=12`; wind station selection uses `observations/met/latest.geojson`, followed by `observations/met/latest.geojson?sensor=…&duration=12` for the selected station's history. Wind history always comes from the same station as the current reading; samples from different stations are never combined. Missing measurements and gaps break chart lines. Requests include the key and Referer required by the service. The provider's page describes a `channelcoast.org` development domain; on 16 October 2026 the live service instead accepted `https://coastalmonitoring.org/` and rejected the old domain. That working Referer is the default here.

To use a personal key, run the executable with the key and its issued Referer in your environment:

```sh
FOMO_API_KEY='your-key' FOMO_API_REFERER='https://your-issued-domain/' \
  dist/Fomo.app/Contents/MacOS/fomo
```

The development key may change. For ongoing use, request your own key through the official page. The app never logs request URLs or API keys, and passes the key to curl on standard input so it does not appear in the process list. If the service rejects the key or Referer, the panel shows the service's own explanation.

Networking uses macOS's `/usr/bin/curl` with system TLS, a connection timeout, total request timeout and a response size cap. Wave retrieval runs alongside wind selection and history retrieval on background threads; AppKit updates return through the main dispatch queue. A tolerant native timer wakes only at the selected refresh interval. No polling loop, bundled browser, or async runtime. A generation counter discards responses for spots or wind sources that you have since changed.

## Development checks

```sh
cargo fmt --check
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo run -- --demo --render-preview /tmp/fomo-previews
```

The last command renders the actual native views in both appearances for visual review without screen recording: conditions (including chart hovers and a state with every reading missing), settings, credits, the forecast page from your saved forecast, and the menu bar at three heights. `--render-swell-preview <dir>` instead renders the conditions page against your saved forecast, reading observations from `snapshot-76.json` and `snapshot-107.json` in that directory (snapshot JSON as printed by `--check-data`).

Notes for coding agents, including a log of recent work, are in `AGENTS.md`. The app is intended for personal local use; the generated bundle is not a notarized distribution.

## Data credits

Real time data displayed on this page are from the Regional Coastal Monitoring Programme, made freely available under the terms of the [Open Government Licence](https://www.nationalarchives.gov.uk/doc/open-government-licence/version/3/). Please note that these are real-time data and are not quality-controlled.

Southwest Regional Coastal Monitoring Programme. Copyright: Teignbridge DC. The full attribution is also accessible from **Data & credits** in the app.


## Local learned forecasts

Click **Forecast** in the conditions panel to see seven days of Hs, peak period and peak direction, with uncertainty ranges. **Refresh forecast** (or Command-R in that view) fetches recent observations and the latest usable operational wave/wind runs, then performs neural mixture inference on the Mac's MPS GPU in a background process. The button is disabled while a refresh is running. Forecast refresh is independent of the observations timer; it does not retrain the model.

The personal worker lives at `~/Documents/BuoyData/deepwater-models`; `FOMO_FORECAST_PROJECT` can override that directory and `FOMO_FORECAST_FILE` can override the JSON path. The worker uses the existing `torch` environment, overridable with `DEEPWATER_PYTHON`. It writes validated `forecasts/fomo-v2.json` atomically. Wave models are ECMWF WAM and NOAA GFS Wave; operational ECMWF IFS winds condition the neural corrections. Inputs use each model's latest usable initialization under the configured eight-hour availability assumption, aligned by forecast valid time. A new run is not necessarily published between refresh clicks, but GPU inference and recent buoy retrieval run again.

Height/period shading is a nominal pointwise 80% range. Hover to inspect each hour, including circular direction uncertainty. Bearings are magnetic FROM and arrows show travel. The current model is experimental: spring/summer training does not establish winter or long-lead coverage. Cached forecasts retain their original issue/valid times and are marked delayed after six hours. Failed updates retain the last forecast and show the error. Demo mode never runs the forecast worker.

For a direct refresh without the UI:

```sh
dist/Fomo.app/Contents/MacOS/fomo --refresh-forecast
```
