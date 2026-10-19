# AGENTS.md

Shared working notes for coding agents (Claude Code, Codex) on Fomo. Read this before starting. When you finish or pause work, update the work log below in the same commit.

## Project

A Rust macOS menu bar app for Cornwall buoy observations, built directly on AppKit through `objc2`. Edition 2024, MSRV 1.85. See `README.md` for user-facing behaviour.

| File | Responsibility |
| --- | --- |
| `src/main.rs` | CLI flags (`--check-data`, `--refresh-forecast`, `--demo`, `--show`, `--render-preview`, `--render-swell-preview`) |
| `src/model.rs` | Settings, domain types, unit conversion, text formatting, menu title |
| `src/data.rs` | Coastal Monitoring API via `/usr/bin/curl`, parsing, QC flags, wind-station choice |
| `src/forecast.rs` | Forecast JSON schema and validation; launches the GPU worker |
| `src/ui/mod.rs` | AppKit app: `State`, the `Delegate` lifecycle (fetching, timers, wake, rendering), main menu, `run` |
| `src/ui/pages.rs` | `Delegate::dashboard`, `settings_view`, `about_view` page builders |
| `src/ui/forecast_page.rs` | Forecast page |
| `src/ui/dashboard.rs` | `Dashboard` view: card drawing and chart hover |
| `src/ui/chart.rs` | `HistoryChart`: wave and wind history geometry and drawing |
| `src/ui/status.rs` | Menu bar readings view (`StatusReadings`, `BearingArrow`) |
| `src/ui/widgets.rs` | Shared helpers: `label`, `button`, `popup`, `top_rect`, colours, arrows |
| `src/ui/preview.rs` | `--render-preview` and `--render-swell-preview` |

Submodules of `ui` start with `use super::*;` and mark items `pub(super)` when a sibling needs them.

The forecast worker is a separate project at `~/Documents/BuoyData/deepwater-models` (not in this repo). Its output `forecasts/fomo-v2.json` must pass `ForecastBundle::validate()`. Today it emits two experts, `ecmwf_wam` and `ncep_gfswave025`.

## Checks

Run all of these before committing:

```sh
cargo fmt --check
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
```

For any UI change, also render and look at the previews:

```sh
cargo run -- --demo --render-preview <scratch dir>
```

`cargo run -- --check-data` makes real API requests and prints parsed observations; use it to verify networking changes.

For a refactor that should not change the UI, render previews from the old and new builds and compare the PNGs with `cmp`. Demo data is relative to the current time, so demo renders made a few seconds apart are byte-identical. Renders of real observations are not, because their positions shift with the clock; compare the underlying data instead.

`--render-preview` writes `conditions-missing-*` with every current reading absent, which is the quickest check of the `—` invariant below.

## Data notes

- Wind comes from shore stations, not buoys. Porthleven has no station. As of 2026-10-19 Penzance (17 km from Porthleven) has sent nothing since about 1 April 2026 and its last record has QC flag 9, so automatic selection gives Porthleven the Perranporth station (33 km). Both spots then show identical wind and wind history. This is expected, not a bug.

## Invariants

- A missing or invalid value displays as `—`, never `0`.
- Bearings are where waves or wind come FROM; arrows point in the direction of travel.
- AppKit objects are touched only on the main thread. Network and worker calls run on background threads and return through the main dispatch queue. Generation counters discard results for spots or stations the user has since changed.
- Never log API keys or request URLs.
- `--demo` never writes settings or caches and never runs the forecast worker.
- Layout uses absolute frames. `top_rect` and `label` take a top-left `y`; AppKit's own coordinates are bottom-left.

## Coordination

- `main` is the released code and is published at https://github.com/gregrmunday/fomo. Work on a feature branch, run the checks, then merge into `main`.
- One commit per work item, with the item number in the subject where there is one (for example `#4: Clear save error after a successful cache write`).
- Before starting an item, mark it `in progress (<agent>)` here and commit, or tell the user, so two agents don't edit the same code at once.
- These repos' local git config commits as `Gregory Munday <100290135+gregrmunday@users.noreply.github.com>`, keeping the user's personal email out of public history. Don't override it.

## Distribution (published 2026-10-19)

Verified on 2026-10-19: `brew install gregrmunday/tap/fomo` poured the bottle on this macOS 27 Mac in 21 s, and `brew test`, the bundle signature and `brew services run`/`stop` all pass. The source-build fallback also works (Fomo itself compiles in 18 s).

Friends install with `brew install gregrmunday/tap/fomo`, then `brew services start fomo`.

| Repository | Visibility | Purpose |
| --- | --- | --- |
| `gregrmunday/fomo` (this repo) | public | App source. The `forecast` branch holds only `fomo-v2.json`, as one commit replaced by every publish. |
| `gregrmunday/homebrew-tap` (`~/Documents/homebrew-tap`) | public | `Formula/fomo.rb`, plus Homebrew's test-bot and pr-pull workflows, which build an Apple Silicon bottle on macOS 15. That bottle also serves macOS 26 and 27. |
| `gregrmunday/fomo-forecast` (`~/Documents/fomo-forecast`) | private | Snapshot of the forecast worker and model; publishes a forecast every three hours. |

How forecasts reach apps:

- `fomo-forecast`'s **Publish forecast** workflow runs the model on CPU (about 1 minute including setup), then force-pushes the result to `fomo`'s `forecast` branch. It uses the write deploy key "fomo-forecast publisher", whose private half is the secret `FOMO_DEPLOY_KEY`.
- Apps without the worker (`forecast::runs_locally()` false) download `https://raw.githubusercontent.com/gregrmunday/fomo/forecast/fomo-v2.json`, validate it and save it as `~/Library/Application Support/Fomo/forecast.json`. The CDN caches it for about 5 minutes.
- The user's own Mac still has the worker, so there Refresh runs it locally on the GPU.
- The research project at `~/Documents/BuoyData/deepwater-models` stays untouched; do not push it anywhere. `fomo-forecast` holds the only published copy of its inference code. After retraining, run `~/Documents/fomo-forecast/sync-from-research.sh`, then commit and push there.

Releasing a new app version:

1. Bump `version` in `Cargo.toml` and `CFBundleShortVersionString`/`CFBundleVersion` in `packaging/Info.plist`. A test fails if they disagree. Build once to update `Cargo.lock`.
2. Merge into `main`, then `git tag -a vX.Y.Z -m "Fomo X.Y.Z"`, push `main` and the tag, and `gh release create vX.Y.Z`.
3. In the tap: `brew bump-formula-pr --url=https://github.com/gregrmunday/fomo/archive/refs/tags/vX.Y.Z.tar.gz gregrmunday/tap/fomo`. When **brew test-bot** passes on the PR, run **brew pr-pull** (`gh workflow run publish.yml -R gregrmunday/homebrew-tap -f pull_request=<n>`) to publish the bottles.

Still open:

- The app and worker ship Coastal Monitoring's published *development* API key with a Referer workaround. The user should request a personal key before sharing widely.
- Apple Silicon Macs older than macOS 15 have no bottle, so they build from source. That needs current Command Line Tools and about 2.5 GB of Homebrew Rust and LLVM; `brew autoremove` reclaims it afterwards.
- Intel Macs are unsupported. Homebrew no longer bottles Rust for Intel (test-bot skips the formula there), so a source build would compile Rust for hours. If a friend needs it, the option is a separately built universal `.app`, which Gatekeeper will warn about because it's unsigned.

## Review status (2026-10-19)

All 18 review items below are done, with one commit each; tests went from 9 to 21 (24 after distribution). Still to check by hand, because they need a real session:

- **#7** Sleep the Mac for longer than the refresh interval, then wake it. Readings should update within about 15 seconds.
- **#11** With the panel open, try ⌘, and ⌘Q from the Forecast, Settings and Credits pages. On Credits, select text and press ⌘C.
- **#13** Click through Forecast, Settings, Credits and Back. The Settings and Data & credits buttons should toggle back to conditions.

## Work log

### Energy audit (2026-10-19)

- **E1 Battery drain investigation.** Status: done. Codex measured the running local 0.1.0 build (PID 4024) at 58–59% CPU, with about 40 minutes of CPU time after an hour. A three-second stack sample showed repeated AppKit status-item replicant snapshots, layer drawing and appearance changes. Switching from all five readings to Compact dropped Fomo to 0.0% CPU and MenuBarAgent from about 14% to 0.8%. The expanded custom status view is the confirmed trigger; the exact AppKit invalidation cycle still needs isolating. Compact is the current workaround. The current source retains this view hierarchy in `src/ui/status.rs` and `Delegate::update_status`; a fix should preserve the expanded layout while avoiding repeated snapshot invalidation. No application code changed. Smaller opportunities: avoid rebuilding hidden popover pages and unchanged status views on refresh; skip forecast hover redraws when the selected hour is unchanged. Local GPU forecast refresh is expensive but manual, and no worker was running during the measurement. Fomo held no sleep assertion.

Review findings from 2026-10-19, worked through in order. Status is `todo`, `in progress (<agent>)`, `done`, or `dropped (<reason>)`.

### Hygiene

- [x] **#1 Version control.** `git init`, baseline commit on `main`, work branch `improvements`, this file. Status: done
- [x] **#2 Dev checks pass.** `cargo fmt` (53 diffs across 4 files) and clippy `too_many_arguments` on `text` and `button` in `forecast_ui.rs`. Status: done

### Bugs

- [x] **#3 Unit label placement.** `Delegate::dashboard` positions the height unit with `height.len()` (bytes, so `—` counts as 3) times 27pt; the wind unit uses a fixed x. Measure the rendered number instead. Status: done
- [x] **#4 Save error never clears.** `Delegate::complete` sets `save_error` on a failed cache write but never clears it on success. Status: done
- [x] **#5 Forecast axis label rounding.** `forecast_ui::view` labels the top gridline with `{high:.0}`; 1.3 m reads as "1". Share the scale computation between drawing and labelling. Status: done
- [x] **#6 Worker error lost.** `forecast::regenerate` reads the first 64 KB of the log instead of the last, and `read_to_string` yields nothing on invalid UTF-8. Status: done

### Robustness and UX

- [x] **#7 Refresh on wake.** Only the repeating timer triggers refreshes. Refresh shortly after `NSWorkspaceDidWakeNotification`, and when the popover opens if the last fetch is older than the interval. Use wall-clock time (`Instant` stops during sleep on macOS). Status: done (wake path needs a manual check: sleep the Mac for longer than the refresh interval, wake, and readings should update within about 15 s)
- [x] **#8 Network error detail.** `data::get` collapses DNS, timeout, TLS and HTTP failures into one message and matches `"key"` anywhere in the body. Classify by curl exit code and HTTP status. Status: done
- [x] **#9 Forecast reload.** `Delegate::complete` re-reads and re-validates the forecast JSON on the main thread every refresh, and load errors are silently dropped. Reload only when the file changes and show load errors. Status: done
- [x] **#10 API key in process arguments.** The key is visible to `ps`. Pass it to curl through `--config -` on stdin. Status: done
- [x] **#11 Shortcuts on every page.** ⌘Q and ⌘, are button key equivalents on the dashboard only. Add an app main menu (also gives ⌘C for selectable text, which LSUIElement apps otherwise lack). Status: done (needs a manual key check: open the panel, then on the Forecast, Settings and Credits pages try ⌘, and ⌘Q; on Credits, select text and press ⌘C)

### Structure

- [x] **#12 Split `ui.rs`** (2,159 lines) into a `ui/` module: app and state, status item, charts, pages, shared widgets, previews. Move `forecast_ui.rs` in as well. Status: done
- [x] **#13 `Page` enum.** Replace `settings`, `about` and `forecast_page` booleans plus the 100/200/300/400 page numbers. Remove the unused `tag` override on `Dashboard`. Status: done (manual check: click through Forecast, Settings, Credits and Back; Settings and Credits buttons toggle back to conditions)
- [x] **#14 Duplication.** `State` built in three places; wind-clearing block in three; status icon built twice; forecast page re-implements widget helpers and uses calibrated rather than sRGB colours. Status: done
- [x] **#15 Forecast page typography.** Match the weights and colours used on the other pages. Status: done

### Tests

- [x] **#16 Coverage.** `ForecastBundle::validate` has no tests. Move prediction merging out of `Dashboard::make` into `forecast.rs` and test it. Cover `age_text`, `stale` for future times, refresh-interval sanitising in `Config::load`, and `State::reload_forecast` (unchanged file skipped, load error surfaced, missing file is not an error). Status: done

### Docs and extras

- [x] **#17 Docs.** README lists DWD EWAM, which neither the app nor the worker uses. "M3 GPU" is hard-coded in the forecast status text. `--render-swell-preview` is undocumented. Status: done
- [x] **#18 Sea temperature.** `Wave.temperature` is parsed, QC-filtered and cached but never shown. Add it to the waves card. Status: done

### Checked and fine

Forecast file on disk passes validation. The worker script `exec`s Python, so the timeout kill reaches it. Settings and cache writes are atomic. Unit conversion constants are correct.
