use super::*;

impl Delegate {
    pub(super) fn dashboard(&self, state: &State) -> Retained<Dashboard> {
        let mtm = self.mtm();
        let root = Dashboard::new(
            mtm,
            state.snapshot.clone(),
            state.config.clone(),
            state.forecast.as_ref(),
        );
        let wave = state.snapshot.latest();
        let wind = state.snapshot.wind.as_ref();
        let config = &state.config;
        label(
            &root,
            mtm,
            "F O M O",
            22.0,
            19.0,
            120.0,
            22.0,
            14.0,
            0.6,
            &ink(),
        );
        label(
            &root,
            mtm,
            if state.demo {
                "DEMO · SAMPLE CONDITIONS"
            } else {
                "CORNWALL · BUOY OBSERVATIONS"
            },
            22.0,
            43.0,
            320.0,
            15.0,
            9.0,
            0.4,
            &muted(),
        );
        button(
            &root,
            self,
            "Forecast",
            sel!(forecast:),
            209.0,
            16.0,
            78.0,
            28.0,
        );
        button(
            &root,
            self,
            "Settings",
            sel!(settings:),
            290.0,
            16.0,
            68.0,
            28.0,
        );

        let spot = popup(
            &root,
            self,
            &["Perranporth", "Porthleven"],
            usize::from(config.spot == Spot::Porthleven),
            1,
            18.0,
            70.0,
            240.0,
            32.0,
        );
        spot.setFont(Some(&NSFont::systemFontOfSize_weight(22.0, 0.5)));
        spot.setBordered(false);
        let refresh = button(
            &root,
            self,
            if state.loading {
                "Updating…"
            } else {
                "↻ Refresh"
            },
            sel!(refresh:),
            269.0,
            73.0,
            89.0,
            26.0,
        );
        refresh.setEnabled(!state.loading);
        refresh.setKeyEquivalent(&NSString::from_str("r"));
        refresh.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);

        let delayed = wave.is_some_and(|w| stale(w.time, 90));
        let status = if state.demo {
            "●  Sample data · preview mode".into()
        } else if let Some(w) = wave {
            format!(
                "{}  Buoy {} · {}",
                if delayed || state.snapshot.wave_error.is_some() {
                    "◷"
                } else {
                    "●"
                },
                w.time.with_timezone(&Local).format("%H:%M"),
                age_text(w.time)
            )
        } else if state.loading {
            "◌  Connecting to the buoy…".into()
        } else {
            "○  Buoy observation unavailable".into()
        };
        let status_color = if delayed || state.snapshot.wave_error.is_some() {
            amber()
        } else {
            muted()
        };
        label(
            &root,
            mtm,
            &status,
            22.0,
            109.0,
            336.0,
            18.0,
            11.0,
            0.0,
            &status_color,
        );

        let white = NSColor::whiteColor();
        let pale = rgb(0.64, 0.79, 0.89);
        label(
            &root, mtm, "WAVES", 40.0, 154.0, 110.0, 16.0, 10.0, 0.5, &pale,
        );
        let sea = format!("SEA {}", temperature_text(wave.and_then(|w| w.temperature)));
        label(
            &root, mtm, &sea, 203.0, 154.0, 140.0, 16.0, 10.0, 0.5, &pale,
        )
        .setAlignment(NSTextAlignment::Right);
        label(
            &root,
            mtm,
            "Significant height · Hs",
            40.0,
            175.0,
            220.0,
            17.0,
            11.0,
            0.0,
            &pale,
        );
        let height = wave
            .and_then(|w| w.height)
            .map(|h| format!("{:.1}", config.height(h)))
            .unwrap_or_else(|| "—".into());
        let height_label = label(
            &root, mtm, &height, 37.0, 195.0, 177.0, 62.0, 48.0, 0.3, &white,
        );
        label(
            &root,
            mtm,
            config.height_unit(),
            (text_end(&height_label) + 3.0).min(204.0),
            223.0,
            40.0,
            25.0,
            17.0,
            0.0,
            &pale,
        );
        let period = wave
            .and_then(|w| w.period)
            .map(|p| format!("{p:.1}s"))
            .unwrap_or_else(|| "—s".into());
        label(
            &root,
            mtm,
            &format!("{period}  peak period"),
            40.0,
            270.0,
            156.0,
            21.0,
            12.0,
            0.2,
            &white,
        );
        let direction = wave.and_then(|w| w.direction);
        label(
            &root,
            mtm,
            &direction
                .map(|d| format!("{:.1}° {}", d.rem_euclid(360.0), compass(d)))
                .unwrap_or_else(|| "—°".into()),
            216.0,
            270.0,
            127.0,
            21.0,
            12.0,
            0.2,
            &white,
        );

        label(
            &root,
            mtm,
            "SWELL · 24 HOURS",
            22.0,
            324.0,
            190.0,
            16.0,
            9.0,
            0.5,
            &muted(),
        );
        let values: Vec<f64> = state
            .snapshot
            .waves
            .iter()
            .filter_map(|w| w.height)
            .collect();
        label(
            &root,
            mtm,
            "● Buoy",
            224.0,
            323.0,
            66.0,
            17.0,
            9.0,
            0.0,
            &wave_color(),
        );
        label(
            &root,
            mtm,
            "● Model",
            294.0,
            323.0,
            64.0,
            17.0,
            9.0,
            0.0,
            &model_color(),
        );
        if values.len() < 2 && root.ivars().charts[0].predictions.is_empty() {
            label(
                &root,
                mtm,
                "The buoy history will appear here",
                70.0,
                369.0,
                260.0,
                20.0,
                12.0,
                0.0,
                &muted(),
            );
        }
        let now = chrono::Utc::now();
        let first = format!(
            "{} · −12h",
            (now - chrono::Duration::hours(12))
                .with_timezone(&Local)
                .format("%H:%M")
        );
        let last = format!(
            "+12h · {}",
            (now + chrono::Duration::hours(12))
                .with_timezone(&Local)
                .format("%H:%M")
        );
        label(
            &root,
            mtm,
            &first,
            22.0,
            419.0,
            110.0,
            16.0,
            9.0,
            0.0,
            &muted(),
        );
        let last_label = label(
            &root,
            mtm,
            &last,
            260.0,
            419.0,
            98.0,
            16.0,
            9.0,
            0.0,
            &muted(),
        );
        last_label.setAlignment(NSTextAlignment::Right);
        let now_label = label(
            &root,
            mtm,
            "Now",
            164.0,
            419.0,
            52.0,
            16.0,
            9.0,
            0.4,
            &muted(),
        );
        now_label.setAlignment(NSTextAlignment::Center);
        let wave_chart = &root.ivars().charts[0];
        let reconstruction = wave_chart
            .predictions
            .iter()
            .any(|p| p.origin == "reconstructed");
        let note = if wave_chart.predictions.is_empty() {
            "Refresh forecast to add your network predictions."
        } else if reconstruction {
            "Dashed past = reconstructed · shading = model 80% range"
        } else {
            "Past model = saved forecasts · shading = model 80% range"
        };
        let note_label = label(
            &root,
            mtm,
            note,
            22.0,
            437.0,
            336.0,
            11.0,
            8.0,
            0.0,
            &muted(),
        );
        let explanation = state.forecast.as_ref().and_then(|b| b.swell_panel.as_ref()).and_then(|p|p.warning.as_deref())
            .unwrap_or("Past dashed predictions were reconstructed using earlier model runs and earlier buoy readings. They were not recorded forecasts at the time. Solid past model lines were saved before their valid time. Hover to inspect values and issue times.");
        note_label.setToolTip(Some(&NSString::from_str(explanation)));

        label(
            &root,
            mtm,
            "WIND",
            39.0,
            462.0,
            100.0,
            16.0,
            10.0,
            0.5,
            &muted(),
        );
        let speed = wind
            .and_then(|w| w.speed)
            .map(|s| format!("{:.0}", config.wind_unit.convert(s)))
            .unwrap_or_else(|| "—".into());
        let speed_label = label(
            &root,
            mtm,
            &speed,
            37.0,
            484.0,
            80.0,
            42.0,
            31.0,
            0.3,
            &ink(),
        );
        label(
            &root,
            mtm,
            config.wind_unit.label(),
            (text_end(&speed_label) + 2.0).min(150.0),
            502.0,
            70.0,
            21.0,
            13.0,
            0.0,
            &muted(),
        );
        label(
            &root,
            mtm,
            &direction_text(wind.and_then(|w| w.direction)),
            219.0,
            483.0,
            125.0,
            25.0,
            14.0,
            0.4,
            &ink(),
        );
        let gust = wind
            .and_then(|w| w.gust)
            .map(|g| format!("Gusts {}", config.wind_text(Some(g))))
            .unwrap_or_else(|| "Gusts —".into());
        label(
            &root,
            mtm,
            &gust,
            197.0,
            512.0,
            145.0,
            18.0,
            11.0,
            0.0,
            &muted(),
        );
        let station = wind
            .map(|w| {
                let distance = distance_km(config.spot.coordinates(), (w.latitude, w.longitude));
                format!(
                    "{} · {:.0} km · {}{}",
                    w.station,
                    distance,
                    age_text(w.time),
                    if stale(w.time, 60) { " · DELAYED" } else { "" }
                )
            })
            .unwrap_or_else(|| "No current wind observation".into());
        let station_color = if wind.is_some_and(|w| stale(w.time, 60)) {
            amber()
        } else {
            muted()
        };
        let station_label = label(
            &root,
            mtm,
            &station,
            39.0,
            541.0,
            304.0,
            18.0,
            10.0,
            0.0,
            &station_color,
        );
        station_label.setToolTip(Some(&NSString::from_str(
            "Wind is measured at the named shore station, which may differ from the buoy location.",
        )));

        label(
            &root,
            mtm,
            "WIND · PAST 12 HOURS",
            22.0,
            584.0,
            190.0,
            16.0,
            9.0,
            0.5,
            &muted(),
        );
        let wind_values: Vec<_> = root.ivars().charts[1]
            .points
            .iter()
            .filter_map(|p| state.snapshot.wind_history[p.sample_index].speed)
            .collect();
        let range = if wind_values.is_empty() {
            "Awaiting observations".into()
        } else {
            let min = wind_values.iter().copied().fold(f64::INFINITY, f64::min);
            let max = wind_values
                .iter()
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            format!(
                "{:.0}–{:.0} {}",
                config.wind_unit.convert(min),
                config.wind_unit.convert(max),
                config.wind_unit.label()
            )
        };
        let range_label = label(
            &root,
            mtm,
            &range,
            208.0,
            583.0,
            150.0,
            17.0,
            10.0,
            0.0,
            &muted(),
        );
        range_label.setAlignment(NSTextAlignment::Right);
        if wind_values.is_empty() {
            label(
                &root,
                mtm,
                if state.loading {
                    "Loading wind history…"
                } else {
                    "Wind history unavailable"
                },
                70.0,
                630.0,
                260.0,
                20.0,
                12.0,
                0.0,
                &muted(),
            );
        }
        let end = wind
            .map(|w| w.time)
            .into_iter()
            .chain(state.snapshot.wind_history.last().map(|w| w.time))
            .max();
        let start = end
            .map(|t| {
                (t - chrono::Duration::hours(12))
                    .with_timezone(&Local)
                    .format("%H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| "12h ago".into());
        let end = end
            .map(|t| t.with_timezone(&Local).format("%H:%M").to_string())
            .unwrap_or_else(|| "Latest".into());
        label(
            &root,
            mtm,
            &start,
            22.0,
            670.0,
            110.0,
            16.0,
            9.0,
            0.0,
            &muted(),
        );
        let end_label = label(
            &root,
            mtm,
            &end,
            260.0,
            670.0,
            98.0,
            16.0,
            9.0,
            0.0,
            &muted(),
        );
        end_label.setAlignment(NSTextAlignment::Right);

        let error = state
            .snapshot
            .wave_error
            .as_ref()
            .or(state.snapshot.wind_error.as_ref())
            .or(state.snapshot.wind_history_error.as_ref())
            .or(state.save_error.as_ref());
        let note = if let Some(error) = error {
            error.clone()
        } else if wave.is_some_and(|w| w.flag.is_some_and(|f| f != 0))
            || wind.is_some_and(|w| w.flag.is_some_and(|f| f != 0))
        {
            "Provider flagged this record; affected values are hidden.".into()
        } else {
            "Combined sea · peak direction · arrows show travel".into()
        };
        let note_color = if error.is_some() { amber() } else { muted() };
        let note_label = label(
            &root,
            mtm,
            &note,
            22.0,
            693.0,
            336.0,
            18.0,
            10.0,
            0.0,
            &note_color,
        );
        note_label.setToolTip(Some(&NSString::from_str(&note)));
        button(
            &root,
            self,
            "Buoy details ↗",
            sel!(source:),
            17.0,
            717.0,
            112.0,
            24.0,
        );
        button(
            &root,
            self,
            "Data & credits",
            sel!(about:),
            139.0,
            717.0,
            109.0,
            24.0,
        );
        button(&root, self, "Quit", sel!(quit:), 307.0, 717.0, 52.0, 24.0);
        root
    }

    pub(super) fn settings_view(&self, state: &State) -> Retained<Dashboard> {
        let mtm = self.mtm();
        let root = Dashboard::plain(mtm, Page::Settings);
        button(&root, self, "‹ Back", sel!(back:), 16.0, 17.0, 68.0, 27.0);
        label(
            &root,
            mtm,
            "Make it yours",
            22.0,
            68.0,
            330.0,
            36.0,
            25.0,
            0.5,
            &ink(),
        );
        label(
            &root,
            mtm,
            "A quick look. Then back to your day.",
            22.0,
            109.0,
            330.0,
            22.0,
            12.0,
            0.0,
            &muted(),
        );
        let c = &state.config;
        let rows: [(&str, &[&str], usize, isize); 6] = [
            (
                "Wave height",
                &["Metres (m)", "Feet (ft)"],
                usize::from(c.wave_unit == WaveUnit::Feet),
                2,
            ),
            (
                "Wind speed",
                &[
                    "Knots (kn)",
                    "Miles per hour",
                    "Kilometres per hour",
                    "Metres per second",
                ],
                match c.wind_unit {
                    WindUnit::Knots => 0,
                    WindUnit::Mph => 1,
                    WindUnit::Kmh => 2,
                    WindUnit::MetresPerSecond => 3,
                },
                3,
            ),
            (
                "Refresh every",
                &["5 minutes", "10 minutes", "15 minutes", "30 minutes"],
                [5, 10, 15, 30]
                    .iter()
                    .position(|n| *n == c.refresh_minutes)
                    .unwrap_or(0),
                4,
            ),
            (
                "Wind station",
                &WIND_STATIONS,
                c.wind_station
                    .as_ref()
                    .and_then(|s| WIND_STATIONS.iter().position(|n| *n == s))
                    .unwrap_or(0),
                5,
            ),
            (
                "Menu bar",
                &["All five readings", "Compact · height, period, wind"],
                usize::from(c.compact),
                6,
            ),
            (
                "Spot",
                &["Perranporth", "Porthleven"],
                usize::from(c.spot == Spot::Porthleven),
                1,
            ),
        ];
        for (i, (title, options, selected, tag)) in rows.iter().enumerate() {
            let y = 156.0 + i as f64 * 58.0;
            label(&root, mtm, title, 22.0, y, 330.0, 18.0, 11.0, 0.4, &muted());
            popup(
                &root,
                self,
                options,
                *selected,
                *tag,
                18.0,
                y + 20.0,
                344.0,
                28.0,
            );
        }
        multiline(
            &root,
            mtm,
            "Automatic uses the closest fresh station within 75 km. Porthleven has no local wind station in this feed; its wind reading may come from another coast.",
            22.0,
            522.0,
            334.0,
            53.0,
            11.0,
            &muted(),
        );
        let note = state.save_error.as_deref().unwrap_or(if state.demo {
            "Preview mode · settings are not saved."
        } else {
            "Changes save automatically on this Mac."
        });
        label(
            &root,
            mtm,
            note,
            22.0,
            589.0,
            334.0,
            18.0,
            10.0,
            0.0,
            &muted(),
        );
        button(
            &root,
            self,
            "Data & API information",
            sel!(about:),
            16.0,
            612.0,
            180.0,
            23.0,
        );
        root
    }

    pub(super) fn about_view(&self, state: &State) -> Retained<Dashboard> {
        let mtm = self.mtm();
        let root = Dashboard::plain(mtm, Page::About);
        button(&root, self, "‹ Back", sel!(back:), 16.0, 17.0, 68.0, 27.0);
        label(
            &root,
            mtm,
            "From sea to screen",
            22.0,
            70.0,
            334.0,
            34.0,
            24.0,
            0.5,
            &ink(),
        );
        label(
            &root,
            mtm,
            "NATIONAL COASTAL MONITORING",
            22.0,
            126.0,
            334.0,
            18.0,
            10.0,
            0.5,
            &muted(),
        );
        multiline(
            &root,
            mtm,
            data::ATTRIBUTION,
            22.0,
            156.0,
            334.0,
            113.0,
            12.0,
            &ink(),
        );
        label(
            &root,
            mtm,
            "READING THE CONDITIONS",
            22.0,
            290.0,
            334.0,
            18.0,
            10.0,
            0.5,
            &muted(),
        );
        multiline(
            &root,
            mtm,
            "Height is the combined significant wave height (Hs), not breaking surf height or an isolated primary swell. Period (Tp) and direction describe the spectral peak. The peak can switch between wave systems.",
            22.0,
            320.0,
            334.0,
            90.0,
            12.0,
            &ink(),
        );
        multiline(
            &root,
            mtm,
            "Bearings name where waves and wind come from. Arrows show where they travel. The provider reports wave bearings relative to magnetic north. Wind speed is converted from m/s.",
            22.0,
            422.0,
            334.0,
            75.0,
            12.0,
            &ink(),
        );
        multiline(
            &root,
            mtm,
            "Waves older than 90 minutes and wind older than 60 minutes are marked delayed. A ! in the menu bar also indicates a connection problem. Last readings remain visible while retrying.",
            22.0,
            506.0,
            334.0,
            75.0,
            11.0,
            &muted(),
        );
        button(
            &root,
            self,
            "API documentation ↗",
            sel!(apiDocs:),
            16.0,
            600.0,
            167.0,
            27.0,
        );
        button(
            &root,
            self,
            &format!("{} buoy ↗", state.config.spot.name()),
            sel!(source:),
            189.0,
            600.0,
            173.0,
            27.0,
        );
        root
    }
}
