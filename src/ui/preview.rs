use super::*;

pub(super) fn render_swell_previews(mtm: MainThreadMarker, dir: &str) {
    std::fs::create_dir_all(dir).expect("create swell preview directory");
    let delegate = Delegate::new(mtm);
    for spot in [Spot::Perranporth, Spot::Porthleven] {
        let config = Config {
            spot,
            ..Config::default()
        };
        let snapshot_path = std::path::Path::new(dir).join(format!("snapshot-{}.json", spot.id()));
        let snapshot: Snapshot = serde_json::from_slice(
            &std::fs::read(snapshot_path).expect("saved live preview observations"),
        )
        .expect("parse live observations");
        let mut state = State::new(config, snapshot, false);
        state.reload_forecast();
        for (theme, name) in unsafe {
            [
                ("light", NSAppearanceNameAqua),
                ("dark", NSAppearanceNameDarkAqua),
            ]
        } {
            let appearance = NSAppearance::appearanceNamed(name).expect("system appearance");
            let view = delegate.dashboard(&state);
            write_preview(
                mtm,
                &view,
                &appearance,
                &format!("{dir}/swell-{}-{theme}.png", spot.id()),
            );
            if let Some(point) = view.ivars().charts[0]
                .points
                .iter()
                .find(|p| p.model_index.is_some() && p.time > chrono::Utc::now())
            {
                view.hover_at(point.position);
                write_preview(
                    mtm,
                    &view,
                    &appearance,
                    &format!("{dir}/swell-{}-hover-{theme}.png", spot.id()),
                );
            }
        }
    }
}

// Render actual AppKit views for visual review without requesting screen recording.
pub(super) fn render_previews(mtm: MainThreadMarker, dir: &str) {
    std::fs::create_dir_all(dir).expect("create preview directory");
    let delegate = Delegate::new(mtm);
    let config = Config::default();
    let mut state = State::new(config.clone(), data::demo(config.spot), true);
    // Every current reading missing: each must render as "—", never as zero.
    let mut missing = state.snapshot.clone();
    if let Some(wave) = missing.waves.last_mut() {
        (wave.height, wave.period, wave.direction) = (None, None, None);
    }
    if let Some(wind) = missing.wind.as_mut() {
        (wind.speed, wind.direction, wind.gust) = (None, None, None);
    }
    // SAFETY: These immutable appearance constants are supplied by linked AppKit.
    let appearances = unsafe {
        [
            ("light", NSAppearanceNameAqua),
            ("dark", NSAppearanceNameDarkAqua),
        ]
    };
    for (theme, name) in appearances {
        let appearance = NSAppearance::appearanceNamed(name).expect("system appearance");
        for page in [
            "conditions",
            "conditions-missing",
            "conditions-hover",
            "conditions-hover-first",
            "conditions-hover-last",
            "wind-hover",
            "wind-hover-first",
            "wind-hover-last",
            "settings",
            "credits",
        ] {
            let view = match page {
                "settings" => delegate.settings_view(&state),
                "credits" => delegate.about_view(&state),
                "conditions-missing" => {
                    let full = std::mem::replace(&mut state.snapshot, missing.clone());
                    let view = delegate.dashboard(&state);
                    state.snapshot = full;
                    view
                }
                _ => delegate.dashboard(&state),
            };
            if page.starts_with("conditions-hover") || page.starts_with("wind-hover") {
                let chart_index = usize::from(page.starts_with("wind-hover"));
                let points = &view.ivars().charts[chart_index].points;
                let index = if page.ends_with("first") {
                    0
                } else if page.ends_with("last") {
                    points.len() - 1
                } else {
                    12
                };
                view.hover_at(points[index].position);
                assert_eq!(view.ivars().hovered.get(), Some((chart_index, index)));
            }
            write_preview(
                mtm,
                &view,
                &appearance,
                &format!("{dir}/{page}-{theme}.png"),
            );
            view.hover_at(NSPoint::new(0.0, 0.0));
            assert!(view.ivars().hovered.get().is_none());
        }
        let forecast = crate::forecast::load_local().ok().flatten();
        let forecast_view = super::forecast_page::view(
            mtm,
            &delegate,
            &state.config,
            forecast.as_ref(),
            false,
            None,
            false,
        );
        write_preview(
            mtm,
            &forecast_view,
            &appearance,
            &format!("{dir}/forecast-{theme}.png"),
        );
        for height in [22.0, 24.0, 32.0] {
            let view = StatusReadings::new(mtm, &state, height);
            assert!(view.hitTest(NSPoint::new(40.0, 10.0)).is_none());
            write_preview(
                mtm,
                &view,
                &appearance,
                &format!("{dir}/menu-bar-{height}-{theme}.png"),
            );
        }
    }
}

pub(super) fn write_preview(
    mtm: MainThreadMarker,
    view: &NSView,
    appearance: &NSAppearance,
    path: &str,
) {
    use objc2_foundation::NSDictionary;
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            view.bounds(),
            NSWindowStyleMask::Borderless,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe {
        window.setReleasedWhenClosed(false);
    }
    window.setAppearance(Some(appearance));
    window.setContentView(Some(view));
    window.displayIfNeeded();
    let bitmap = view
        .bitmapImageRepForCachingDisplayInRect(view.bounds())
        .expect("allocate preview bitmap");
    view.cacheDisplayInRect_toBitmapImageRep(view.bounds(), &bitmap);
    // SAFETY: Empty encoding properties have no type-dependent entries.
    let png = unsafe {
        bitmap.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .expect("encode PNG");
    assert!(png.writeToFile_atomically(&NSString::from_str(path), true));
    println!("{path}");
}
