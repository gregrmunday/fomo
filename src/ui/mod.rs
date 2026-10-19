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

mod chart;
mod dashboard;
mod forecast_page;
mod pages;
mod preview;
mod status;
mod widgets;

use chart::*;
use dashboard::*;
use status::*;
use widgets::*;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Conditions,
    Settings,
    About,
    Forecast,
}

thread_local! { static APP: OnceCell<Retained<Delegate>> = const { OnceCell::new() }; }

struct State {
    config: Config,
    snapshot: Snapshot,
    generation: u64,
    loading: bool,
    // Wall-clock time of the last completed fetch, for refreshing after sleep.
    last_fetch: Option<chrono::DateTime<chrono::Utc>>,
    page: Page,
    demo: bool,
    save_error: Option<String>,
    forecast: Option<crate::forecast::ForecastBundle>,
    forecast_error: Option<String>,
    forecast_loading: bool,
    forecast_generation: u64,
}

impl State {
    fn new(config: Config, snapshot: Snapshot, demo: bool) -> Self {
        Self {
            config,
            snapshot,
            generation: 0,
            loading: false,
            last_fetch: None,
            page: Page::Conditions,
            demo,
            save_error: None,
            forecast: None,
            forecast_error: None,
            forecast_loading: false,
            forecast_generation: 0,
        }
    }

    fn reload_forecast(&mut self) {
        self.reload_forecast_from(&crate::forecast::output_path());
    }

    // Reads the saved forecast only when the file has changed since it was loaded,
    // so the regular observation refresh costs one stat call.
    fn reload_forecast_from(&mut self, path: &std::path::Path) {
        if self.demo || self.forecast_loading {
            return;
        }
        let loaded = self.forecast.as_ref().and_then(|b| b.modified);
        if loaded.is_some() && crate::forecast::modified(path) == loaded {
            return;
        }
        match crate::forecast::load_from(path) {
            Ok(Some(bundle)) => {
                self.forecast = Some(bundle);
                self.forecast_error = None;
            }
            Ok(None) => {}
            Err(error) => self.forecast_error = Some(format!("Saved forecast unreadable: {error}")),
        }
    }
}

struct Ivars {
    state: RefCell<State>,
    status: OnceCell<Retained<NSStatusItem>>,
    popover: OnceCell<Retained<NSPopover>>,
    controller: OnceCell<Retained<NSViewController>>,
    timer: RefCell<Option<Retained<NSTimer>>>,
    // The page whose views are currently installed in the popover.
    shown_page: Cell<Option<Page>>,
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
                if let Some(image) = wave_icon() {
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
            NSApplication::sharedApplication(mtm).setMainMenu(Some(&main_menu(self)));
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
        fn settings_action(&self, _sender: Option<&AnyObject>) { self.toggle_page(Page::Settings); }

        #[unsafe(method(about:))]
        fn about_action(&self, _sender: Option<&AnyObject>) { self.toggle_page(Page::About); }

        #[unsafe(method(back:))]
        fn back_action(&self, _sender: Option<&AnyObject>) { self.show(Page::Conditions); }

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
                    state.snapshot.clear_wind();
                    refetch = true;
                }
                2 => state.config.wave_unit = if index == 0 { WaveUnit::Metres } else { WaveUnit::Feet },
                3 => state.config.wind_unit = [WindUnit::Knots, WindUnit::Mph, WindUnit::Kmh, WindUnit::MetresPerSecond][index.min(3)],
                4 => state.config.refresh_minutes = [5,10,15,30][index.min(3)],
                5 => {
                    state.config.wind_station = if index == 0 { None } else { Some(WIND_STATIONS[index.min(5)].into()) };
                    state.snapshot.clear_wind();
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
            if self.ivars().state.borrow().page != Page::Settings { self.render(); }
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
            self.ivars().state.borrow_mut().reload_forecast();
            self.show(Page::Forecast);
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
            cache.clear_wind();
            cache
        };
        let mut state = State::new(config, snapshot, demo);
        state.reload_forecast();
        let this = Self::alloc(mtm).set_ivars(Ivars {
            state: RefCell::new(state),
            status: OnceCell::new(),
            popover: OnceCell::new(),
            controller: OnceCell::new(),
            timer: RefCell::new(None),
            shown_page: Cell::new(None),
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
        state.reload_forecast();
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
                            Err(error) => {
                                state.forecast_error = Some(format!("Update failed: {error}"))
                            }
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
        if state.page == Page::Settings && self.ivars().shown_page.get() == Some(Page::Settings) {
            return;
        }
        let root: Retained<NSView> = match state.page {
            Page::Forecast => forecast_page::view(
                self.mtm(),
                self,
                &state.config,
                state.forecast.as_ref(),
                state.forecast_loading,
                state.forecast_error.as_deref(),
                state.demo,
            ),
            Page::Settings => self.settings_view(&state).into_super(),
            Page::About => self.about_view(&state).into_super(),
            Page::Conditions => self.dashboard(&state).into_super(),
        };
        controller.setView(&root);
        self.ivars().shown_page.set(Some(state.page));
    }

    fn show(&self, page: Page) {
        self.ivars().state.borrow_mut().page = page;
        self.render();
    }

    // The Settings and Credits buttons (and ⌘,) toggle back to the conditions page.
    fn toggle_page(&self, page: Page) {
        let current = self.ivars().state.borrow().page;
        self.show(if current == page {
            Page::Conditions
        } else {
            page
        });
    }
}

// The menu bar never shows this menu (Fomo is an accessory app), but AppKit still
// matches its key equivalents while the panel is key, so shortcuts work on every page.
fn main_menu(delegate: &Delegate) -> Retained<NSMenu> {
    let mtm = delegate.mtm();
    let submenu = |title: &str, items: &[(&str, Sel, &str, bool)]| {
        let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
        for (title, action, key, to_delegate) in items {
            // SAFETY: Each action takes one sender argument. Delegate actions are defined on
            // Delegate, which outlives the menu; copy:/selectAll: go to the first responder.
            unsafe {
                let item = NSMenuItem::initWithTitle_action_keyEquivalent(
                    NSMenuItem::alloc(mtm),
                    &NSString::from_str(title),
                    Some(*action),
                    &NSString::from_str(key),
                );
                if *to_delegate {
                    item.setTarget(Some(delegate));
                }
                menu.addItem(&item);
            }
        }
        let holder = NSMenuItem::new(mtm);
        holder.setSubmenu(Some(&menu));
        holder
    };
    let bar = NSMenu::new(mtm);
    bar.addItem(&submenu(
        "Fomo",
        &[
            ("Settings…", sel!(settings:), ",", true),
            ("Quit Fomo", sel!(quit:), "q", true),
        ],
    ));
    // Lets ⌘C copy the selectable credits and notes text.
    bar.addItem(&submenu(
        "Edit",
        &[
            ("Copy", sel!(copy:), "c", false),
            ("Select All", sel!(selectAll:), "a", false),
        ],
    ));
    bar
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
        preview::render_swell_previews(mtm, &dir);
        return;
    }
    if let Some(dir) = std::env::args()
        .skip_while(|a| a != "--render-preview")
        .nth(1)
    {
        preview::render_previews(mtm, &dir);
        return;
    }
    let delegate = Delegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    APP.with(|slot| {
        let _ = slot.set(delegate);
    });
    app.run();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn forecast_reload_skips_unchanged_files_and_reports_bad_ones() {
        let dir = std::env::temp_dir().join(format!("fomo-reload-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("forecast.json");
        let mut state = State::new(Config::default(), Snapshot::default(), false);

        // No file yet: nothing to show, and not an error.
        state.reload_forecast_from(&path);
        assert!(state.forecast.is_none() && state.forecast_error.is_none());

        std::fs::write(&path, "{").unwrap();
        state.reload_forecast_from(&path);
        let error = state.forecast_error.clone().unwrap();
        assert!(error.starts_with("Saved forecast unreadable"), "{error}");

        let valid = serde_json::to_vec(&crate::forecast::tests::sample()).unwrap();
        std::fs::write(&path, valid).unwrap();
        state.reload_forecast_from(&path);
        assert!(state.forecast.is_some() && state.forecast_error.is_none());

        // Unchanged: not re-read, so this marker survives.
        state.forecast_error = Some("marker".into());
        state.reload_forecast_from(&path);
        assert_eq!(state.forecast_error.as_deref(), Some("marker"));

        // A newer modification time means the file is read again.
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(60))
            .unwrap();
        drop(file);
        state.reload_forecast_from(&path);
        assert_eq!(state.forecast_error, None);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
