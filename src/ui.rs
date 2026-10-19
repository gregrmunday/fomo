use crate::forecast::{ForecastBundle, SwellPoint};
use crate::{data, model::*};
use chrono::Local;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2::{
    rc::Retained,
    runtime::{AnyObject, ProtocolObject, Sel},
};
use objc2_app_kit::*;
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSRectEdge,
    NSSize, NSString, NSTimer, NSURL,
};
use std::cell::{Cell, OnceCell, RefCell};

const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 744.0;
const WIND_STATIONS: [&str; 6] = [
    "Automatic · nearest fresh station",
    "Perranporth",
    "Penzance",
    "Port Isaac",
    "Looe Bay",
    "Bude",
];

thread_local! { static APP: OnceCell<Retained<Delegate>> = const { OnceCell::new() }; }

struct State {
    config: Config,
    snapshot: Snapshot,
    generation: u64,
    loading: bool,
    // Wall-clock time of the last completed fetch, for refreshing after sleep.
    last_fetch: Option<chrono::DateTime<chrono::Utc>>,
    settings: bool,
    about: bool,
    demo: bool,
    save_error: Option<String>,
    forecast: Option<crate::forecast::ForecastBundle>,
    forecast_error: Option<String>,
    forecast_loading: bool,
    forecast_page: bool,
    forecast_generation: u64,
}

struct Ivars {
    state: RefCell<State>,
    status: OnceCell<Retained<NSStatusItem>>,
    popover: OnceCell<Retained<NSPopover>>,
    controller: OnceCell<Retained<NSViewController>>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    current_page: Cell<isize>,
    status_readings: RefCell<Option<Retained<StatusReadings>>>,
}

define_class!(
    // All AppKit objects and mutable UI state live on the main thread.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = Ivars]
    struct Delegate;

    unsafe impl NSObjectProtocol for Delegate {}
    unsafe impl NSApplicationDelegate for Delegate {
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_launch(&self, _notification: &NSNotification) {
            let mtm = self.mtm();
            let status = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
            status.setAutosaveName(Some(&NSString::from_str("FomoConditions")));
            if let Some(button) = status.button(mtm) {
                unsafe {
                    button.setTarget(Some(self));
                    button.setAction(Some(sel!(toggle:)));
                }
                button.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(11.0, 0.0)));
                if let Some(image) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    &NSString::from_str("water.waves"), Some(&NSString::from_str("Fomo surf conditions"))) {
                    image.setTemplate(true);
                    image.setSize(NSSize::new(17.0, 16.0));
                    button.setImage(Some(&image));
                    button.setImagePosition(NSCellImagePosition::ImageLeft);
                }
            }
            let _ = self.ivars().status.set(status);
            let controller = NSViewController::new(mtm);
            let popover = NSPopover::new(mtm);
            popover.setContentViewController(Some(&controller));
            popover.setContentSize(NSSize::new(WIDTH, HEIGHT));
            popover.setBehavior(NSPopoverBehavior::Transient);
            let _ = self.ivars().controller.set(controller);
            let _ = self.ivars().popover.set(popover);
            // SAFETY: didWake: is defined below and takes the notification. The delegate
            // lives for the whole process, so the observer is never left dangling.
            unsafe {
                NSWorkspace::sharedWorkspace().notificationCenter().addObserver_selector_name_object(
                    self, sel!(didWake:), Some(NSWorkspaceDidWakeNotification), None);
            }
            self.render();
            self.schedule();
            self.refresh();
            if std::env::args().any(|a| a == "--show") { self.toggle(); }
        }
    }

    impl Delegate {
        #[unsafe(method(toggle:))]
        fn toggle_action(&self, _sender: Option<&AnyObject>) { self.toggle(); }

        #[unsafe(method(refresh:))]
        fn refresh_action(&self, _sender: Option<&AnyObject>) { self.refresh(); }

        #[unsafe(method(settings:))]
        fn settings_action(&self, _sender: Option<&AnyObject>) {
            let mut state = self.ivars().state.borrow_mut();
            state.forecast_page = false;
            state.settings = !state.settings;
            state.about = false;
            drop(state);
            self.render();
        }

        #[unsafe(method(about:))]
        fn about_action(&self, _sender: Option<&AnyObject>) {
            let mut state = self.ivars().state.borrow_mut();
            state.forecast_page = false;
            state.about = !state.about;
            state.settings = false;
            drop(state);
            self.render();
        }

        #[unsafe(method(back:))]
        fn back_action(&self, _sender: Option<&AnyObject>) {
            let mut state = self.ivars().state.borrow_mut();
            state.forecast_page = false;
            state.settings = false;
            state.about = false;
            drop(state);
            self.render();
        }

        #[unsafe(method(changed:))]
        fn changed(&self, sender: &NSPopUpButton) {
            let mut state = self.ivars().state.borrow_mut();
            let index = sender.indexOfSelectedItem() as usize;
            let mut refetch = false;
            match sender.tag() {
                1 => {
                    state.config.spot = if index == 0 { Spot::Perranporth } else { Spot::Porthleven };
                    state.snapshot = if state.demo { data::demo(state.config.spot) } else { Snapshot::load(state.config.spot) };
                    // A cached wind observation may have used a different source.
                    state.snapshot.wind = None;
                    state.snapshot.wind_history.clear();
                    state.snapshot.wind_history_error = None;
                    refetch = true;
                }
                2 => state.config.wave_unit = if index == 0 { WaveUnit::Metres } else { WaveUnit::Feet },
                3 => state.config.wind_unit = [WindUnit::Knots, WindUnit::Mph, WindUnit::Kmh, WindUnit::MetresPerSecond][index.min(3)],
                4 => state.config.refresh_minutes = [5,10,15,30][index.min(3)],
                5 => {
                    state.config.wind_station = if index == 0 { None } else { Some(WIND_STATIONS[index.min(5)].into()) };
                    state.snapshot.wind = None;
                    state.snapshot.wind_history.clear();
                    state.snapshot.wind_history_error = None;
                    state.snapshot.wind_error = None;
                    refetch = true;
                }
                6 => state.config.compact = index == 1,
                _ => return,
            }
            if !state.demo { state.save_error = state.config.save().err(); }
            if refetch { state.generation += 1; state.loading = false; }
            drop(state);
            self.schedule();
            // Keep controls in place while a settings popup is being used.
            self.update_status();
            if !self.ivars().state.borrow().settings { self.render(); }
            if refetch { self.refresh(); }
        }

        #[unsafe(method(source:))]
        fn source(&self, _sender: Option<&AnyObject>) {
            let spot = self.ivars().state.borrow().config.spot;
            open_url(&format!("https://coastalmonitoring.org/realtimedata/?chart={}", spot.id()));
        }

        #[unsafe(method(apiDocs:))]
        fn api_docs(&self, _sender: Option<&AnyObject>) {
            open_url("https://coastalmonitoring.org/ccoresources/api/");
        }

        #[unsafe(method(forecast:))]
        fn forecast_action(&self, _sender: Option<&AnyObject>) {
            let mut state = self.ivars().state.borrow_mut();
            state.forecast_page = true;
            state.settings = false;
            state.about = false;
            if !state.demo && !state.forecast_loading {
                if let Ok(bundle) = crate::forecast::load_local() { state.forecast = Some(bundle); }
            }
            drop(state);
            self.render();
        }

        #[unsafe(method(refreshForecast:))]
        fn refresh_forecast_action(&self, _sender: Option<&AnyObject>) { self.refresh_forecast(); }

        #[unsafe(method(didWake:))]
        fn did_wake(&self, _notification: &NSNotification) {
            // Give Wi-Fi a moment to reconnect before fetching.
            // SAFETY: refreshIfDue: is defined below and takes the timer as its sender.
            unsafe {
                NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                    10.0, self, sel!(refreshIfDue:), None, false);
            }
        }

        #[unsafe(method(refreshIfDue:))]
        fn refresh_if_due_action(&self, _sender: Option<&AnyObject>) { self.refresh_if_due(); }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let config = Config::load();
        let demo = std::env::args().any(|a| a == "--demo");
        let snapshot = if demo {
            data::demo(config.spot)
        } else {
            let mut cache = Snapshot::load(config.spot);
            // Re-evaluate wind selection each launch; old source preferences are not trusted.
            cache.wind = None;
            cache.wind_history.clear();
            cache.wind_history_error = None;
            cache
        };
        let this = Self::alloc(mtm).set_ivars(Ivars {
            state: RefCell::new(State {
                config,
                snapshot,
                generation: 0,
                loading: false,
                last_fetch: None,
                settings: false,
                about: false,
                demo,
                save_error: None,
                forecast: if demo {
                    None
                } else {
                    crate::forecast::load_local().ok()
                },
                forecast_error: None,
                forecast_loading: false,
                forecast_page: false,
                forecast_generation: 0,
            }),
            status: OnceCell::new(),
            popover: OnceCell::new(),
            controller: OnceCell::new(),
            timer: RefCell::new(None),
            current_page: Cell::new(0),
            status_readings: RefCell::new(None),
        });
        // SAFETY: NSObject init has this signature; ivars are initialized above.
        unsafe { msg_send![super(this), init] }
    }

    fn toggle(&self) {
        let Some(popover) = self.ivars().popover.get() else {
            return;
        };
        if popover.isShown() {
            popover.close();
            return;
        }
        self.refresh_if_due();
        self.render();
        if let Some(button) = self.ivars().status.get().and_then(|s| s.button(self.mtm())) {
            #[allow(deprecated)]
            NSApplication::sharedApplication(self.mtm()).activateIgnoringOtherApps(true);
            popover.showRelativeToRect_ofView_preferredEdge(
                button.bounds(),
                &button,
                NSRectEdge::MinY,
            );
            if let Some(window) = popover
                .contentViewController()
                .and_then(|c| c.view().window())
            {
                window.setAcceptsMouseMovedEvents(true);
                window.makeKeyWindow();
            }
        }
    }

    fn schedule(&self) {
        if let Some(timer) = self.ivars().timer.borrow_mut().take() {
            timer.invalidate();
        }
        if self.ivars().state.borrow().demo {
            return;
        }
        let seconds = self.ivars().state.borrow().config.refresh_minutes as f64 * 60.0;
        // SAFETY: The selector is defined above, takes the timer sender, and runs on the main run loop.
        let timer = unsafe {
            NSTimer::scheduledTimerWithTimeInterval_target_selector_userInfo_repeats(
                seconds,
                self,
                sel!(refresh:),
                None,
                true,
            )
        };
        timer.setTolerance(15.0);
        *self.ivars().timer.borrow_mut() = Some(timer);
    }

    fn refresh(&self) {
        let mut state = self.ivars().state.borrow_mut();
        if state.demo {
            state.snapshot = data::demo(state.config.spot);
            drop(state);
            self.render();
            return;
        }
        if state.loading {
            return;
        }
        state.loading = true;
        state.generation += 1;
        let generation = state.generation;
        let config = state.config.clone();
        drop(state);
        self.render();
        std::thread::spawn(move || {
            let result = data::fetch(&config);
            dispatch2::DispatchQueue::main().exec_async(move || {
                APP.with(|app| {
                    if let Some(app) = app.get() {
                        app.complete(generation, result);
                    }
                });
            });
        });
    }

    // The repeating timer does not catch up after sleep, so wake and opening the panel
    // fetch when the last completed fetch is older than the refresh interval.
    fn refresh_if_due(&self) {
        let state = self.ivars().state.borrow();
        let due = !state.demo && fetch_due(state.last_fetch, state.config.refresh_minutes);
        drop(state);
        if due {
            self.refresh();
        }
    }

    fn complete(&self, generation: u64, mut result: Snapshot) {
        let mut state = self.ivars().state.borrow_mut();
        if state.generation != generation {
            return;
        }
        state.last_fetch = Some(chrono::Utc::now());
        if result.wave_error.is_some() {
            result.waves = state.snapshot.waves.clone();
        }
        // Keep the last observation on network failure, with a visible error.
        if result.wind_error.is_some() {
            result.wind = state.snapshot.wind.clone();
            result.wind_history = state.snapshot.wind_history.clone();
        } else if result.wind_history_error.is_some()
            && result.wind.as_ref().map(|w| &w.station)
                == state.snapshot.wind.as_ref().map(|w| &w.station)
        {
            result.wind_history = state.snapshot.wind_history.clone();
        }
        state.snapshot = result;
        if let Ok(bundle) = crate::forecast::load_local() {
            state.forecast = Some(bundle);
        }
        state.loading = false;
        // A successful write shows storage has recovered, so clear any earlier failure.
        state.save_error = state.snapshot.save(state.config.spot).err();
        drop(state);
        self.render();
    }

    fn refresh_forecast(&self) {
        let mut state = self.ivars().state.borrow_mut();
        if state.demo || state.forecast_loading {
            return;
        }
        state.forecast_loading = true;
        state.forecast_error = None;
        state.forecast_generation += 1;
        let generation = state.forecast_generation;
        drop(state);
        self.render();
        std::thread::spawn(move || {
            let result = crate::forecast::regenerate();
            dispatch2::DispatchQueue::main().exec_async(move || {
                APP.with(|app| {
                    if let Some(app) = app.get() {
                        let mut state = app.ivars().state.borrow_mut();
                        if state.forecast_generation != generation {
                            return;
                        }
                        state.forecast_loading = false;
                        match result {
                            Ok(bundle) => state.forecast = Some(bundle),
                            Err(error) => state.forecast_error = Some(error),
                        }
                        drop(state);
                        app.render();
                    }
                });
            });
        });
    }

    fn update_status(&self) {
        let state = self.ivars().state.borrow();
        if let Some(button) = self.ivars().status.get().and_then(|s| s.button(self.mtm())) {
            let title = if state.demo {
                format!("DEMO {}", menu_title(&state.config, &state.snapshot, false))
            } else {
                menu_title(&state.config, &state.snapshot, state.loading)
            };
            if let Some(old) = self.ivars().status_readings.borrow_mut().take() {
                old.removeFromSuperview();
            }
            if state.config.compact {
                self.ivars().status.get().unwrap().setLength(-1.0);
                button.setImage(wave_icon().as_deref());
                button.setTitle(&NSString::from_str(&format!(" {title}")));
            } else {
                button.setTitle(&NSString::from_str(""));
                button.setImage(None);
                let view =
                    StatusReadings::new(self.mtm(), &state, button.bounds().size.height.max(22.0));
                self.ivars()
                    .status
                    .get()
                    .unwrap()
                    .setLength(view.frame().size.width);
                button.addSubview(&view);
                *self.ivars().status_readings.borrow_mut() = Some(view);
            }
            button.setAccessibilityLabel(Some(&NSString::from_str(&title)));
            let wave_time = state
                .snapshot
                .latest()
                .map(|w| format!("Waves {}", age_text(w.time)))
                .unwrap_or_else(|| "Waves unavailable".into());
            let wind_time = state
                .snapshot
                .wind
                .as_ref()
                .map(|w| format!("Wind: {} · {}", w.station, age_text(w.time)))
                .unwrap_or_else(|| "Wind unavailable".into());
            let tip = format!(
                "{}\n{wave_time}\n{wind_time}\nHs = combined significant wave height. Period/direction = spectral peak.\nCompass bearings are FROM; arrows point in the direction of travel.\n! = delayed/unavailable data or connection error. Click for details.",
                state.config.spot.name()
            );
            button.setToolTip(Some(&NSString::from_str(&tip)));
        }
    }

    fn render(&self) {
        self.update_status();
        let state = self.ivars().state.borrow();
        let Some(controller) = self.ivars().controller.get() else {
            return;
        };
        // Settings controls stay alive during a background refresh so keyboard focus is preserved.
        if state.settings && self.ivars().current_page.get() == 200 {
            return;
        }
        let root: Retained<NSView> = if state.forecast_page {
            crate::forecast_ui::view(
                self.mtm(),
                self,
                &state.config,
                state.forecast.as_ref(),
                state.forecast_loading,
                state.forecast_error.as_deref(),
                state.demo,
            )
        } else if state.settings {
            self.settings_view(&state).into_super()
        } else if state.about {
            self.about_view(&state).into_super()
        } else {
            self.dashboard(&state).into_super()
        };
        controller.setView(&root);
        self.ivars().current_page.set(if state.forecast_page {
            400
        } else if state.settings {
            200
        } else if state.about {
            300
        } else {
            100
        });
    }

    fn dashboard(&self, state: &State) -> Retained<Dashboard> {
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
        let settings = button(
            &root,
            self,
            "Settings",
            sel!(settings:),
            290.0,
            16.0,
            68.0,
            28.0,
        );
        settings.setKeyEquivalent(&NSString::from_str(","));
        settings.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);

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
            &rgb(0.03, 0.56, 0.64),
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
            &rgb(0.57, 0.32, 0.79),
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
        let quit = button(&root, self, "Quit", sel!(quit:), 307.0, 717.0, 52.0, 24.0);
        quit.setKeyEquivalent(&NSString::from_str("q"));
        quit.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
        root
    }

    fn settings_view(&self, state: &State) -> Retained<Dashboard> {
        let mtm = self.mtm();
        let root = Dashboard::plain(mtm, 200);
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

    fn about_view(&self, state: &State) -> Retained<Dashboard> {
        let mtm = self.mtm();
        let root = Dashboard::plain(mtm, 300);
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

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Option<f64>]
    struct BearingArrow;
    unsafe impl NSObjectProtocol for BearingArrow {}
    impl BearingArrow {
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) {
            if let Some(direction) = *self.ivars() {
                let size = self.bounds().size;
                draw_arrow_sized(size.width/2.0,size.height/2.0,direction,size.width*0.34,1.15,&ink());
            }
        }
    }
);

impl BearingArrow {
    fn new(mtm: MainThreadMarker, direction: Option<f64>, size: f64) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(direction);
        // SAFETY: NSView's designated initializer, on the main thread.
        unsafe {
            msg_send![super(this), initWithFrame: NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(size,size))]
        }
    }
}

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Cell<Option<f64>>]
    struct StatusReadings;
    unsafe impl NSObjectProtocol for StatusReadings {}
    impl StatusReadings {
        // Let the status bar button handle clicks, highlights and its tooltip.
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> *mut NSView { std::ptr::null_mut() }

        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) {
            if let Some(x) = self.ivars().get() {
                let center = self.bounds().size.height/2.0;
                let path = NSBezierPath::new();
                path.moveToPoint(NSPoint::new(x,center-6.5));
                path.lineToPoint(NSPoint::new(x,center+6.5));
                NSColor::tertiaryLabelColor().setStroke();
                path.setLineWidth(0.75); path.setLineCapStyle(NSLineCapStyle::Round); path.stroke();
            }
        }
    }
);

impl StatusReadings {
    fn new(mtm: MainThreadMarker, state: &State, height: f64) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Cell::new(None));
        // SAFETY: NSView's designated initializer, on the main thread.
        let view: Retained<Self> = unsafe {
            msg_send![super(this), initWithFrame: NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(160.0,height))]
        };
        let wave = state.snapshot.latest();
        let wind = state.snapshot.wind.as_ref();
        let title = menu_title(&state.config, &state.snapshot, state.loading);
        let marker = if title.ends_with('!') {
            " !"
        } else if state.loading {
            " ·"
        } else {
            ""
        };
        let spot = if state.demo {
            "DEMO"
        } else {
            state.config.spot.short()
        };
        let period = wave
            .and_then(|w| w.period)
            .map(|p| format!("{p:.0}s"))
            .unwrap_or_else(|| "—s".into());
        let wave_bottom = format!(
            "{} {period}",
            state.config.height_text(wave.and_then(|w| w.height))
        );
        let wind_bottom = state.config.wind_text(wind.and_then(|w| w.speed));
        let top_y = (height / 2.0).floor() - 1.0;
        let bottom_y = top_y - 10.0;
        let mut x = 25.0;
        let groups = [
            (
                format!("{spot}{marker}"),
                wave.and_then(|w| w.direction),
                wave_bottom,
            ),
            (String::new(), wind.and_then(|w| w.direction), wind_bottom),
        ];
        for (i, (prefix, direction, bottom)) in groups.into_iter().enumerate() {
            let upper = direction_row(mtm, &prefix, direction);
            view.addSubview(&upper);
            let lower = status_label(&view, mtm, &bottom, 10.0, 0.4);
            lower.sizeToFit();
            let width = upper
                .frame()
                .size
                .width
                .max(lower.frame().size.width)
                .ceil()
                + 1.0;
            upper.setFrameOrigin(NSPoint::new(
                x + (width - upper.frame().size.width) / 2.0,
                top_y,
            ));
            lower.setFrame(NSRect::new(
                NSPoint::new(x, bottom_y),
                NSSize::new(width, 12.0),
            ));
            x += width;
            if i == 0 {
                view.ivars().set(Some(x + 4.0));
                x += 8.0;
            }
        }
        if let Some(image) = wave_icon() {
            let icon = NSImageView::initWithFrame(
                NSImageView::alloc(mtm),
                NSRect::new(
                    NSPoint::new(5.0, (height - 16.0) / 2.0),
                    NSSize::new(16.0, 16.0),
                ),
            );
            icon.setImage(Some(&image));
            view.addSubview(&icon);
        }
        view.setFrameSize(NSSize::new(x + 5.0, height));
        view
    }
}

fn direction_row(mtm: MainThreadMarker, prefix: &str, direction: Option<f64>) -> Retained<NSView> {
    let row = NSView::initWithFrame(
        NSView::alloc(mtm),
        NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(100.0, 12.0)),
    );
    let mut x = 0.0;
    if !prefix.is_empty() {
        let label = status_label(&row, mtm, prefix, 9.0, 0.0);
        label.sizeToFit();
        let width = label.frame().size.width;
        label.setFrame(NSRect::new(NSPoint::new(x, 0.0), NSSize::new(width, 12.0)));
        x += width + 2.0;
    }
    if direction.is_some() {
        let arrow = BearingArrow::new(mtm, direction, 12.0);
        arrow.setFrameOrigin(NSPoint::new(x, 0.0));
        row.addSubview(&arrow);
        x += 13.0;
    }
    let text = direction.map(compass).unwrap_or("—");
    let label = status_label(&row, mtm, text, 9.0, 0.0);
    label.sizeToFit();
    let width = label.frame().size.width;
    label.setFrame(NSRect::new(NSPoint::new(x, 0.0), NSSize::new(width, 12.0)));
    row.setFrameSize(NSSize::new(x + width, 12.0));
    row
}

fn wave_icon() -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str("water.waves"),
        Some(&NSString::from_str("Fomo surf conditions")),
    )?;
    image.setTemplate(true);
    image.setSize(NSSize::new(17.0, 16.0));
    Some(image)
}

fn status_label(
    parent: &NSView,
    mtm: MainThreadMarker,
    text: &str,
    size: f64,
    weight: f64,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
        size, weight,
    )));
    label.setAlignment(NSTextAlignment::Center);
    parent.addSubview(&label);
    label
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HistoryKind {
    Waves,
    Wind,
}

struct ChartPoint {
    sample_index: usize,
    time: chrono::DateTime<chrono::Utc>,
    position: NSPoint,
    model_index: Option<usize>,
}

struct HistoryChart {
    kind: HistoryKind,
    points: Vec<ChartPoint>,
    plot: NSRect,
    predictions: Vec<SwellPoint>,
    start: chrono::DateTime<chrono::Utc>,
    end: chrono::DateTime<chrono::Utc>,
    low: f64,
    high: f64,
}

impl HistoryChart {
    fn new(snapshot: &Snapshot, kind: HistoryKind, predictions: &[SwellPoint]) -> Self {
        let plot = match kind {
            HistoryKind::Waves => top_rect(22.0, 351.0, 336.0, 60.0),
            HistoryKind::Wind => top_rect(22.0, 608.0, 336.0, 56.0),
        };
        let samples: Vec<_> = match kind {
            HistoryKind::Waves => snapshot
                .waves
                .iter()
                .enumerate()
                .map(|(i, w)| (i, w.time, w.height))
                .collect(),
            HistoryKind::Wind => snapshot
                .wind_history
                .iter()
                .enumerate()
                .map(|(i, w)| (i, w.time, w.speed))
                .collect(),
        };
        let end = match kind {
            HistoryKind::Waves => Some(chrono::Utc::now() + chrono::Duration::hours(12)),
            HistoryKind::Wind => snapshot
                .wind
                .as_ref()
                .map(|w| w.time)
                .into_iter()
                .chain(snapshot.wind_history.last().map(|w| w.time))
                .max(),
        };
        let mut chart = Self {
            kind,
            points: Vec::new(),
            plot,
            predictions: Vec::new(),
            start: chrono::Utc::now(),
            end: chrono::Utc::now(),
            low: 0.0,
            high: 1.0,
        };
        let Some(end) = end else {
            return chart;
        };
        let duration = if kind == HistoryKind::Waves {
            24.0
        } else {
            12.0
        };
        let start = end - chrono::Duration::hours(duration as i64);
        chart.start = start;
        chart.end = end;
        if kind == HistoryKind::Waves {
            chart.predictions = predictions
                .iter()
                .filter(|p| p.valid_at_utc >= start && p.valid_at_utc <= end)
                .cloned()
                .collect();
        }
        let values: Vec<_> = samples
            .into_iter()
            .filter(|(_, t, _)| *t >= start && *t <= end)
            .filter_map(|(i, t, v)| v.map(|v| (i, t, v)))
            .collect();
        let padding = if kind == HistoryKind::Waves {
            0.15
        } else {
            0.5
        };
        let low = (values
            .iter()
            .map(|p| p.2)
            .chain(chart.predictions.iter().map(|p| p.hs_m.p10))
            .fold(f64::INFINITY, f64::min)
            - padding)
            .max(0.0);
        let high = values
            .iter()
            .map(|p| p.2)
            .chain(chart.predictions.iter().map(|p| p.hs_m.p90))
            .fold(f64::NEG_INFINITY, f64::max)
            + padding;
        chart.low = if low.is_finite() { low } else { 0.0 };
        chart.high = if high.is_finite() {
            high.max(chart.low + 0.1)
        } else {
            1.0
        };
        let low = chart.low;
        let high = chart.high;
        chart.points = values
            .into_iter()
            .map(|(sample_index, time, value)| ChartPoint {
                sample_index,
                time,
                model_index: None,
                position: NSPoint::new(
                    plot.origin.x
                        + (time - start).num_seconds() as f64 / (duration * 3600.0)
                            * plot.size.width,
                    plot.origin.y + (value - low) / (high - low) * plot.size.height,
                ),
            })
            .collect();
        for (index, prediction) in chart.predictions.iter().enumerate() {
            chart.points.push(ChartPoint {
                sample_index: index,
                time: prediction.valid_at_utc,
                position: chart.position(prediction.valid_at_utc, prediction.hs_m.p50),
                model_index: Some(index),
            });
        }
        chart.points.sort_by_key(|p| p.time);
        chart
    }

    fn position(&self, time: chrono::DateTime<chrono::Utc>, value: f64) -> NSPoint {
        NSPoint::new(
            self.plot.origin.x
                + (time - self.start).num_seconds() as f64
                    / (self.end - self.start).num_seconds() as f64
                    * self.plot.size.width,
            self.plot.origin.y
                + (value - self.low) / (self.high - self.low) * self.plot.size.height,
        )
    }

    fn tracking_rect(&self) -> NSRect {
        NSRect::new(
            NSPoint::new(self.plot.origin.x - 4.0, self.plot.origin.y - 4.0),
            NSSize::new(self.plot.size.width + 8.0, self.plot.size.height + 8.0),
        )
    }

    fn color(&self) -> Retained<NSColor> {
        match self.kind {
            HistoryKind::Waves => rgb(0.03, 0.56, 0.64),
            HistoryKind::Wind => NSColor::systemBlueColor(),
        }
    }

    fn draw(&self) {
        if self.points.is_empty() {
            return;
        }
        for i in 0..3 {
            let line = NSBezierPath::new();
            let y = self.plot.origin.y + i as f64 * self.plot.size.height / 2.0;
            line.moveToPoint(NSPoint::new(self.plot.origin.x, y));
            line.lineToPoint(NSPoint::new(self.plot.origin.x + self.plot.size.width, y));
            NSColor::separatorColor().setStroke();
            line.setLineWidth(0.5);
            line.stroke();
        }
        if self.kind == HistoryKind::Waves {
            let x = self.plot.origin.x + self.plot.size.width / 2.0;
            let now = NSBezierPath::new();
            now.moveToPoint(NSPoint::new(x, self.plot.origin.y));
            now.lineToPoint(NSPoint::new(x, self.plot.origin.y + self.plot.size.height));
            NSColor::tertiaryLabelColor().setStroke();
            now.setLineWidth(0.8);
            now.stroke();
            for pair in self.predictions.windows(2) {
                let a = &pair[0];
                let b = &pair[1];
                if b.valid_at_utc - a.valid_at_utc > chrono::Duration::minutes(90) {
                    continue;
                }
                let band = NSBezierPath::new();
                band.moveToPoint(self.position(a.valid_at_utc, a.hs_m.p10));
                band.lineToPoint(self.position(b.valid_at_utc, b.hs_m.p10));
                band.lineToPoint(self.position(b.valid_at_utc, b.hs_m.p90));
                band.lineToPoint(self.position(a.valid_at_utc, a.hs_m.p90));
                band.closePath();
                NSColor::colorWithCalibratedRed_green_blue_alpha(0.57, 0.32, 0.79, 0.12).setFill();
                band.fill();
                let line = NSBezierPath::new();
                let from = self.position(a.valid_at_utc, a.hs_m.p50);
                let to = self.position(b.valid_at_utc, b.hs_m.p50);
                line.moveToPoint(from);
                if a.origin == "reconstructed" {
                    line.lineToPoint(NSPoint::new(
                        from.x + (to.x - from.x) * 0.55,
                        from.y + (to.y - from.y) * 0.55,
                    ));
                } else {
                    line.lineToPoint(to);
                }
                rgb(0.57, 0.32, 0.79).setStroke();
                line.setLineWidth(2.0);
                line.stroke();
            }
        }
        let path = NSBezierPath::new();
        let mut previous: Option<&ChartPoint> = None;
        let max_gap = if self.kind == HistoryKind::Waves {
            45
        } else {
            20
        };
        for point in self.points.iter().filter(|p| p.model_index.is_none()) {
            let connected = previous.is_some_and(|old| {
                point.sample_index == old.sample_index + 1
                    && (point.time - old.time).num_minutes() <= max_gap
            });
            if connected {
                path.lineToPoint(point.position);
            } else {
                path.moveToPoint(point.position);
            }
            previous = Some(point);
        }
        self.color().setStroke();
        path.setLineWidth(2.5);
        path.stroke();
        if let Some(point) = self.points.iter().rev().find(|p| p.model_index.is_none()) {
            self.draw_dot(point.position, 3.5);
        }
    }

    fn draw_dot(&self, point: NSPoint, radius: f64) {
        let dot = NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
            NSPoint::new(point.x - radius, point.y - radius),
            NSSize::new(radius * 2.0, radius * 2.0),
        ));
        self.color().setFill();
        dot.fill();
    }

    fn bubble(&self, point: NSPoint) -> NSRect {
        let (width, height) = if self.kind == HistoryKind::Waves {
            (290.0, 54.0)
        } else {
            (236.0, 40.0)
        };
        NSRect::new(
            NSPoint::new(
                (point.x - width / 2.0).clamp(22.0, 358.0 - width),
                (point.y + 12.0).min(self.plot.origin.y + self.plot.size.height + 7.0 - height),
            ),
            NSSize::new(width, height),
        )
    }
}

struct CanvasState {
    snapshot: Snapshot,
    page: isize,
    config: Config,
    charts: Vec<HistoryChart>,
    hovered: Cell<Option<(usize, usize)>>,
    hover_label: OnceCell<Retained<NSTextField>>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = CanvasState]
    struct Dashboard;
    unsafe impl NSObjectProtocol for Dashboard {}
    impl Dashboard {
        #[unsafe(method(tag))]
        fn view_tag(&self) -> isize { self.ivars().page }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) { self.hover_at(self.convertPoint_fromView(event.locationInWindow(),None)); }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) { self.hover_at(self.convertPoint_fromView(event.locationInWindow(),None)); }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            // Adjacent tracking areas can deliver exit/enter events in either order.
            let location = self.window().map(|w| self.convertPoint_fromView(w.mouseLocationOutsideOfEventStream(),None));
            if let Some(location) = location { self.hover_at(location); } else { self.set_hover(None); }
        }

        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) {
            NSColor::windowBackgroundColor().setFill(); NSBezierPath::bezierPathWithRect(self.bounds()).fill();
            if self.ivars().page != 100 { return; }
            fill_round(top_rect(22.0,140.0,336.0,168.0),16.0,&rgb(0.045,0.15,0.23));
            fill_round(top_rect(22.0,450.0,336.0,118.0),14.0,&NSColor::quaternaryLabelColor());
            let ring = NSBezierPath::bezierPathWithOvalInRect(top_rect(252.0,177.0,64.0,64.0));
            rgb(0.20,0.42,0.53).setStroke(); ring.setLineWidth(1.0); ring.stroke();
            if let Some(d) = self.ivars().snapshot.latest().and_then(|w| w.direction) {
                draw_arrow(284.0,HEIGHT-209.0,d,&rgb(0.40,0.88,0.86));
            }
            if let Some(d) = self.ivars().snapshot.wind.as_ref().and_then(|w| w.direction) {
                draw_arrow_sized(204.0,HEIGHT-496.0,d,7.0,1.8,&ink());
            }
            for chart in &self.ivars().charts { chart.draw(); }
            if let Some((chart_index,point_index)) = self.ivars().hovered.get() {
                let chart = &self.ivars().charts[chart_index];
                let point = chart.points[point_index].position;
                let guide = NSBezierPath::new();
                guide.moveToPoint(NSPoint::new(point.x,chart.plot.origin.y));
                guide.lineToPoint(NSPoint::new(point.x,chart.plot.origin.y+chart.plot.size.height));
                NSColor::tertiaryLabelColor().setStroke(); guide.setLineWidth(1.0); guide.stroke();
                chart.draw_dot(point,4.0);
                let rect = chart.bubble(point);
                fill_round(rect,6.0,&NSColor::controlBackgroundColor());
                let border = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect,6.0,6.0);
                NSColor::separatorColor().setStroke(); border.setLineWidth(0.5); border.stroke();
            }
        }
    }
);

impl Dashboard {
    fn new(
        mtm: MainThreadMarker,
        snapshot: Snapshot,
        config: Config,
        forecast: Option<&ForecastBundle>,
    ) -> Retained<Self> {
        let view = Self::make(mtm, snapshot, config, 100, forecast);
        let label = NSTextField::wrappingLabelWithString(&NSString::from_str(""), mtm);
        label.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
            11.0, 0.4,
        )));
        label.setAlignment(NSTextAlignment::Center);
        label.setHidden(true);
        view.addSubview(&label);
        let _ = view.ivars().hover_label.set(label);
        for chart in &view.ivars().charts {
            // SAFETY: Dashboard implements the tracking selectors; userInfo is absent.
            let tracking = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    chart.tracking_rect(),
                    NSTrackingAreaOptions::MouseMoved
                        | NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveInKeyWindow,
                    Some(&view),
                    None,
                )
            };
            view.addTrackingArea(&tracking);
        }
        view
    }
    fn plain(mtm: MainThreadMarker, page: isize) -> Retained<Self> {
        Self::make(mtm, Snapshot::default(), Config::default(), page, None)
    }
    fn make(
        mtm: MainThreadMarker,
        snapshot: Snapshot,
        config: Config,
        page: isize,
        forecast: Option<&ForecastBundle>,
    ) -> Retained<Self> {
        let mut predictions = std::collections::BTreeMap::new();
        if let Some(bundle) = forecast {
            if let Some(series) = bundle.swell_panel.as_ref().and_then(|p| {
                p.forecasts
                    .iter()
                    .find(|s| s.station_id == config.spot.id())
            }) {
                for point in &series.points {
                    predictions.insert(point.valid_at_utc, point.clone());
                }
            }
            if let Some(series) = bundle.for_station(config.spot.id()) {
                for point in &series.points {
                    if bundle.generated_at_utc <= point.valid_at_utc {
                        predictions.insert(
                            point.valid_at_utc,
                            SwellPoint {
                                valid_at_utc: point.valid_at_utc,
                                hs_m: point.hs_m.clone(),
                                tp_s: point.tp_s.clone(),
                                direction: point.direction.clone(),
                                issued_at_utc: bundle.issued_at_utc,
                                generated_at_utc: bundle.generated_at_utc,
                                model_version: bundle.model_version.clone(),
                                origin: "forecast".into(),
                            },
                        );
                    }
                }
            }
        }
        let predictions: Vec<_> = predictions.into_values().collect();
        let charts = vec![
            HistoryChart::new(&snapshot, HistoryKind::Waves, &predictions),
            HistoryChart::new(&snapshot, HistoryKind::Wind, &[]),
        ];
        let this = Self::alloc(mtm).set_ivars(CanvasState {
            snapshot,
            page,
            config,
            charts,
            hovered: Cell::new(None),
            hover_label: OnceCell::new(),
        });
        // SAFETY: NSView's designated initializer, on the main thread.
        unsafe {
            msg_send![super(this), initWithFrame: NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(WIDTH,HEIGHT))]
        }
    }
    fn hover_at(&self, location: NSPoint) {
        let selection = self
            .ivars()
            .charts
            .iter()
            .enumerate()
            .find_map(|(chart_index, chart)| {
                let plot = chart.tracking_rect();
                if location.x < plot.origin.x
                    || location.x > plot.origin.x + plot.size.width
                    || location.y < plot.origin.y
                    || location.y > plot.origin.y + plot.size.height
                {
                    return None;
                }
                chart
                    .points
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        let distance = |p: &ChartPoint| {
                            (p.position.x - location.x).abs()
                                + (p.position.y - location.y).abs() * 0.2
                        };
                        distance(a).total_cmp(&distance(b))
                    })
                    .filter(|(_, p)| (p.position.x - location.x).abs() <= 9.0)
                    .map(|(i, _)| (chart_index, i))
            });
        self.set_hover(selection);
    }
    fn set_hover(&self, selection: Option<(usize, usize)>) {
        if self.ivars().hovered.replace(selection) == selection {
            return;
        }
        if let Some(label) = self.ivars().hover_label.get() {
            label.setHidden(selection.is_none());
            if let Some((chart_index, point_index)) = selection {
                let chart = &self.ivars().charts[chart_index];
                let point = &chart.points[point_index];
                let config = &self.ivars().config;
                let value = match chart.kind {
                    HistoryKind::Waves => {
                        if let Some(index) = point.model_index {
                            let p = &chart.predictions[index];
                            format!(
                                "{} · {} {} ({:.1}–{:.1}{})\nTp {:.1}s · {} {:.0}°\nIssued {} · {}",
                                p.valid_at_utc.with_timezone(&Local).format("%H:%M"),
                                if p.origin == "reconstructed" {
                                    "Reconstructed"
                                } else {
                                    "Model"
                                },
                                config.height_text(Some(p.hs_m.p50)),
                                config.height(p.hs_m.p10),
                                config.height(p.hs_m.p90),
                                config.height_unit(),
                                p.tp_s.p50,
                                compass(p.direction.from_deg),
                                p.direction.from_deg,
                                p.issued_at_utc.with_timezone(&Local).format("%d %b %H:%M"),
                                if p.origin == "reconstructed" {
                                    "not saved at the time"
                                } else {
                                    "saved forecast"
                                }
                            )
                        } else {
                            let wave = &self.ivars().snapshot.waves[point.sample_index];
                            format!(
                                "{} · Buoy {}\nTp {} · {}",
                                wave.time.with_timezone(&Local).format("%H:%M"),
                                config.height_text(wave.height),
                                wave.period
                                    .map(|p| format!("{p:.1}s"))
                                    .unwrap_or("—".into()),
                                direction_text(wave.direction)
                            )
                        }
                    }
                    HistoryKind::Wind => {
                        let wind = &self.ivars().snapshot.wind_history[point.sample_index];
                        let speed = wind
                            .speed
                            .map(|s| {
                                format!(
                                    "{:.1}{}",
                                    config.wind_unit.convert(s),
                                    config.wind_unit.label()
                                )
                            })
                            .unwrap_or_else(|| "—".into());
                        format!(
                            "{} · {speed}\nGust {} · {}",
                            wind.time.with_timezone(&Local).format("%H:%M"),
                            config.wind_text(wind.gust),
                            direction_text(wind.direction)
                        )
                    }
                };
                label.setStringValue(&NSString::from_str(&value));
                label.setToolTip(
                    point
                        .model_index
                        .map(|index| {
                            NSString::from_str(&format!(
                                "Model {}",
                                chart.predictions[index].model_version
                            ))
                        })
                        .as_deref(),
                );
                let mut rect = chart.bubble(point.position);
                rect.origin.y += 3.0;
                rect.size.height -= 5.0;
                label.setFrame(rect);
            }
        }
        self.setNeedsDisplay(true);
    }
}

fn top_rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, HEIGHT - y - h), NSSize::new(w, h))
}
// Right edge of a single-line label's rendered text, for placing a trailing unit.
fn text_end(label: &NSTextField) -> f64 {
    label.frame().origin.x + label.sizeThatFits(NSSize::new(10_000.0, 10_000.0)).width
}
fn rgb(r: f64, g: f64, b: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0)
}
fn ink() -> Retained<NSColor> {
    NSColor::labelColor()
}
fn muted() -> Retained<NSColor> {
    NSColor::secondaryLabelColor()
}
fn amber() -> Retained<NSColor> {
    NSColor::systemOrangeColor()
}
fn fill_round(rect: NSRect, radius: f64, color: &NSColor) {
    color.setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius).fill();
}
fn draw_arrow(x: f64, y: f64, from: f64, color: &NSColor) {
    draw_arrow_sized(x, y, from, 18.0, 3.0, color);
}
fn draw_arrow_sized(x: f64, y: f64, from: f64, radius: f64, width: f64, color: &NSColor) {
    let (dx, dy) = travel_vector(from);
    let tip = NSPoint::new(x + dx * radius, y + dy * radius);
    let path = NSBezierPath::new();
    path.moveToPoint(NSPoint::new(x - dx * radius, y - dy * radius));
    path.lineToPoint(tip);
    path.moveToPoint(NSPoint::new(
        tip.x - dx * radius * 0.61 + dy * radius * 0.44,
        tip.y - dy * radius * 0.61 - dx * radius * 0.44,
    ));
    path.lineToPoint(tip);
    path.lineToPoint(NSPoint::new(
        tip.x - dx * radius * 0.61 - dy * radius * 0.44,
        tip.y - dy * radius * 0.61 + dx * radius * 0.44,
    ));
    color.setStroke();
    path.setLineWidth(width);
    path.setLineCapStyle(NSLineCapStyle::Round);
    path.stroke();
}

#[allow(clippy::too_many_arguments)]
fn label(
    parent: &NSView,
    mtm: MainThreadMarker,
    text: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    size: f64,
    weight: f64,
    color: &NSColor,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFrame(top_rect(x, y, w, h));
    label.setFont(Some(&NSFont::systemFontOfSize_weight(size, weight)));
    label.setTextColor(Some(color));
    label.setToolTip(Some(&NSString::from_str(text)));
    parent.addSubview(&label);
    label
}

#[allow(clippy::too_many_arguments)]
fn multiline(
    parent: &NSView,
    mtm: MainThreadMarker,
    text: &str,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    size: f64,
    color: &NSColor,
) {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    label.setFrame(top_rect(x, y, w, h));
    label.setFont(Some(&NSFont::systemFontOfSize(size)));
    label.setTextColor(Some(color));
    label.setSelectable(true);
    parent.addSubview(&label);
}

#[allow(clippy::too_many_arguments)]
fn button(
    parent: &NSView,
    target: &Delegate,
    title: &str,
    action: Sel,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Retained<NSButton> {
    // SAFETY: All selectors passed here are defined on Delegate with one sender argument.
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(
            &NSString::from_str(title),
            Some(target),
            Some(action),
            target.mtm(),
        )
    };
    button.setFrame(top_rect(x, y, w, h));
    button.setBezelStyle(NSBezelStyle::Push);
    button.setBordered(false);
    button.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    parent.addSubview(&button);
    button
}

#[allow(clippy::too_many_arguments)]
fn popup(
    parent: &NSView,
    target: &Delegate,
    options: &[&str],
    selected: usize,
    tag: isize,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
) -> Retained<NSPopUpButton> {
    let popup = NSPopUpButton::initWithFrame_pullsDown(
        NSPopUpButton::alloc(target.mtm()),
        top_rect(x, y, w, h),
        false,
    );
    for option in options {
        popup.addItemWithTitle(&NSString::from_str(option));
    }
    popup.selectItemAtIndex(selected as isize);
    popup.setTag(tag);
    // SAFETY: changed: expects exactly this NSPopUpButton sender. Delegate outlives all controls.
    unsafe {
        popup.setTarget(Some(target));
        popup.setAction(Some(sel!(changed:)));
    }
    parent.addSubview(&popup);
    popup
}

fn open_url(url: &str) {
    if let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

pub fn run() {
    let mtm = MainThreadMarker::new().expect("Fomo must launch on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    if let Some(dir) = std::env::args()
        .skip_while(|a| a != "--render-swell-preview")
        .nth(1)
    {
        render_swell_previews(mtm, &dir);
        return;
    }
    if let Some(dir) = std::env::args()
        .skip_while(|a| a != "--render-preview")
        .nth(1)
    {
        render_previews(mtm, &dir);
        return;
    }
    let delegate = Delegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    APP.with(|slot| {
        let _ = slot.set(delegate);
    });
    app.run();
}

fn render_swell_previews(mtm: MainThreadMarker, dir: &str) {
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
        let state = State {
            snapshot,
            config,
            generation: 0,
            loading: false,
            last_fetch: None,
            settings: false,
            about: false,
            demo: false,
            save_error: None,
            forecast: crate::forecast::load_local().ok(),
            forecast_error: None,
            forecast_loading: false,
            forecast_page: false,
            forecast_generation: 0,
        };
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
fn render_previews(mtm: MainThreadMarker, dir: &str) {
    std::fs::create_dir_all(dir).expect("create preview directory");
    let delegate = Delegate::new(mtm);
    let config = Config::default();
    let mut state = State {
        snapshot: data::demo(config.spot),
        config,
        generation: 0,
        loading: false,
        last_fetch: None,
        settings: false,
        about: false,
        demo: true,
        save_error: None,
        forecast: None,
        forecast_error: None,
        forecast_loading: false,
        forecast_page: false,
        forecast_generation: 0,
    };
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
        let forecast = crate::forecast::load_local().ok();
        let forecast_view = crate::forecast_ui::view(
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

fn write_preview(mtm: MainThreadMarker, view: &NSView, appearance: &NSAppearance, path: &str) {
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
