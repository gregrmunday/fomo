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

## Invariants

- A missing or invalid value displays as `—`, never `0`.
- Bearings are where waves or wind come FROM; arrows point in the direction of travel.
- AppKit objects are touched only on the main thread. Network and worker calls run on background threads and return through the main dispatch queue. Generation counters discard results for spots or stations the user has since changed.
- Never log API keys or request URLs.
- `--demo` never writes settings or caches and never runs the forecast worker.
- Layout uses absolute frames. `top_rect` and `label` take a top-left `y`; AppKit's own coordinates are bottom-left.

## Coordination

- `main` holds the untouched baseline. Work happens on the `improvements` branch.
- One commit per work-log item, with the item number in the subject (for example `#4: Clear save error after a successful cache write`).
- Before starting an item, set its status to `in progress (<agent>)` and commit or tell the user, so two agents don't edit the same code at once.
- Items 12 to 15 restructure `ui.rs`. Avoid starting other UI items in parallel with them.

## Work log

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

- [ ] **#16 Coverage.** `ForecastBundle::validate` has no tests. Move prediction merging out of `Dashboard::make` into `forecast.rs` and test it. Cover `age_text`, `stale` for future times, refresh-interval sanitising in `Config::load`, and `State::reload_forecast` (unchanged file skipped, load error surfaced, missing file is not an error). Status: in progress (Claude)

### Docs and extras

- [ ] **#17 Docs.** README lists DWD EWAM, which neither the app nor the worker uses. "M3 GPU" is hard-coded in the forecast status text. `--render-swell-preview` is undocumented. Status: todo
- [ ] **#18 Sea temperature.** `Wave.temperature` is parsed, QC-filtered and cached but never shown. Add it to the waves card. Status: todo

### Checked and fine

Forecast file on disk passes validation. The worker script `exec`s Python, so the timeout kill reaches it. Settings and cache writes are atomic. Unit conversion constants are correct.
