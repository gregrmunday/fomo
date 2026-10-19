use super::*;
use crate::forecast::{BuoyForecast, ForecastPoint};

// Horizontal extent of the forecast charts.
const CHART_LEFT: f64 = 40.0;
const CHART_WIDTH: f64 = 306.0;
struct ForecastIvars {
    buoy: Option<BuoyForecast>,
    config: Config,
    selected: Cell<usize>,
    readout: OnceCell<Retained<NSTextField>>,
}
define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = ForecastIvars]
    struct ForecastCanvas;
    unsafe impl NSObjectProtocol for ForecastCanvas {}
    impl ForecastCanvas {
        #[unsafe(method(drawRect:))]
        fn draw(&self, _dirty: NSRect) {
            NSColor::windowBackgroundColor().setFill();
            NSBezierPath::bezierPathWithRect(self.bounds()).fill();
            let Some(buoy) = &self.ivars().buoy else { return; };
            for (channel, top, height) in [(0, 220.0, 106.0), (1, 386.0, 106.0)] {
                let points = &buoy.points;
                let config = &self.ivars().config;
                let value = |p: &ForecastPoint, quantile: usize| channel_value(config, p, channel, quantile);
                let high = scale_max(config, buoy, channel);
                let pos = |i: usize, v: f64| NSPoint::new(CHART_LEFT+i as f64/(points.len()-1) as f64*CHART_WIDTH,
                    HEIGHT-top-height+v/high*height);
                for grid in 0..=2 {
                    let line = NSBezierPath::new();
                    line.moveToPoint(NSPoint::new(CHART_LEFT, HEIGHT-top-height+grid as f64/2.0*height));
                    line.lineToPoint(NSPoint::new(CHART_LEFT+CHART_WIDTH, HEIGHT-top-height+grid as f64/2.0*height));
                    NSColor::separatorColor().setStroke(); line.setLineWidth(0.5); line.stroke();
                }
                let band = NSBezierPath::new();
                band.moveToPoint(pos(0, value(&points[0], 0)));
                for (i,p) in points.iter().enumerate().skip(1) { band.lineToPoint(pos(i,value(p,0))); }
                for (i,p) in points.iter().enumerate().rev() { band.lineToPoint(pos(i,value(p,2))); }
                band.closePath();
                wave_color().colorWithAlphaComponent(0.17).setFill(); band.fill();
                let path = NSBezierPath::new();
                for (i,p) in points.iter().enumerate() {
                    if i == 0 { path.moveToPoint(pos(i,value(p,1))); } else { path.lineToPoint(pos(i,value(p,1))); }
                }
                wave_color().setStroke(); path.setLineWidth(2.0); path.stroke();
                let index = self.ivars().selected.get().min(points.len()-1);
                let p = pos(index, value(&points[index],1));
                let guide = NSBezierPath::new();
                guide.moveToPoint(NSPoint::new(p.x,HEIGHT-top-height));
                guide.lineToPoint(NSPoint::new(p.x,HEIGHT-top));
                NSColor::tertiaryLabelColor().setStroke(); guide.setLineWidth(0.8); guide.stroke();
                wave_color().setFill(); NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(p.x-3.,p.y-3.),NSSize::new(6.,6.))).fill();
            }
            for day in 0..7 {
                let index = (day * 24).min(buoy.points.len()-1);
                let direction = buoy.points[index].direction.from_deg;
                let (dx,dy) = travel_vector(direction);
                let x = chart_x(buoy, index);
                let y = HEIGHT-582.;
                let tip = NSPoint::new(x+dx*10.,y+dy*10.);
                let line = NSBezierPath::new();
                line.moveToPoint(NSPoint::new(x-dx*8.,y-dy*8.)); line.lineToPoint(tip);
                line.moveToPoint(NSPoint::new(tip.x-dx*5.-dy*3.,tip.y-dy*5.+dx*3.)); line.lineToPoint(tip);
                line.lineToPoint(NSPoint::new(tip.x-dx*5.+dy*3.,tip.y-dy*5.-dx*3.));
                wave_color().setStroke(); line.setLineWidth(1.8); line.stroke();
            }
        }
        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) { self.select_at(self.convertPoint_fromView(event.locationInWindow(), None)); }
        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, event: &NSEvent) { self.select_at(self.convertPoint_fromView(event.locationInWindow(), None)); }
    }
);
impl ForecastCanvas {
    fn new(mtm: MainThreadMarker, buoy: Option<BuoyForecast>, config: Config) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ForecastIvars {
            buoy,
            config,
            selected: Cell::new(0),
            readout: OnceCell::new(),
        });
        unsafe {
            msg_send![super(this), initWithFrame: NSRect::new(NSPoint::new(0.,0.),NSSize::new(WIDTH,HEIGHT))]
        }
    }
    fn select_at(&self, point: NSPoint) {
        let Some(buoy) = &self.ivars().buoy else {
            return;
        };
        let from_top = HEIGHT - point.y;
        if !(200.0..=620.0).contains(&from_top) {
            return;
        }
        let index = (((point.x - CHART_LEFT) / CHART_WIDTH).clamp(0., 1.)
            * (buoy.points.len() - 1) as f64)
            .round() as usize;
        self.ivars().selected.set(index);
        self.update_readout();
        self.setNeedsDisplay(true);
    }
    fn update_readout(&self) {
        let Some(buoy) = &self.ivars().buoy else {
            return;
        };
        let p = &buoy.points[self.ivars().selected.get().min(buoy.points.len() - 1)];
        let config = &self.ivars().config;
        let d = &p.direction;
        let text = format!(
            "{} · {} {:.0}°\nHs {} ({:.1}–{:.1}{}) · Tp {:.1}s ({:.1}–{:.1}s)\nDirection range ±{:.0}°{}",
            p.valid_at_utc
                .with_timezone(&Local)
                .format("%a %d %b, %H:%M"),
            compass(d.from_deg),
            d.from_deg,
            config.height_text(Some(p.hs_m.p50)),
            config.height(p.hs_m.p10),
            config.height(p.hs_m.p90),
            config.height_unit(),
            p.tp_s.p50,
            p.tp_s.p10,
            p.tp_s.p90,
            d.interval80.half_width_deg,
            if d.interval80.wraps_north {
                " · crosses north"
            } else if d.interval80.full_circle {
                " · all directions"
            } else {
                ""
            }
        );
        if let Some(label) = self.ivars().readout.get() {
            label.setStringValue(&NSString::from_str(&text));
        }
    }
}
// A forecast quantile (0 = p10, 1 = p50, 2 = p90) in display units. Channel 0 is Hs, 1 is Tp.
fn channel_value(config: &Config, p: &ForecastPoint, channel: usize, quantile: usize) -> f64 {
    let q = if channel == 0 { &p.hs_m } else { &p.tp_s };
    let raw = [q.p10, q.p50, q.p90][quantile];
    if channel == 0 {
        config.height(raw)
    } else {
        raw
    }
}
// Top of a chart's scale: the largest p90 plus headroom. Drawing and the axis label share it.
fn scale_max(config: &Config, buoy: &BuoyForecast, channel: usize) -> f64 {
    buoy.points
        .iter()
        .map(|p| channel_value(config, p, channel, 2))
        .fold(0.0, f64::max)
        .max(1.0)
        * 1.08
}
fn axis_text(value: f64) -> String {
    if value < 10.0 {
        format!("{value:.1}")
    } else {
        format!("{value:.0}")
    }
}
// X position of a forecast hour on the charts.
fn chart_x(buoy: &BuoyForecast, index: usize) -> f64 {
    CHART_LEFT + index as f64 / (buoy.points.len() - 1) as f64 * CHART_WIDTH
}
pub fn view(
    mtm: MainThreadMarker,
    target: &Delegate,
    config: &Config,
    bundle: Option<&ForecastBundle>,
    loading: bool,
    error: Option<&str>,
    demo: bool,
) -> Retained<NSView> {
    let buoy = bundle
        .and_then(|b| b.for_station(config.spot.id()))
        .cloned();
    let view = ForecastCanvas::new(mtm, buoy, config.clone());
    // Header follows the other pages: back at top left, then the spot as the title,
    // with its refresh action alongside as on the conditions page.
    button(
        &view,
        target,
        "‹ Observations",
        sel!(back:),
        16.,
        17.,
        112.,
        27.,
    );
    label(
        &view,
        mtm,
        "FORECAST · NEXT SEVEN DAYS",
        22.,
        52.,
        320.,
        15.,
        9.,
        0.4,
        &muted(),
    );
    let name = view
        .ivars()
        .buoy
        .as_ref()
        .map(|b| b.name.as_str())
        .unwrap_or(config.spot.name());
    label(&view, mtm, name, 22., 70., 185., 32., 22., 0.5, &ink());
    let refresh = button(
        &view,
        target,
        if loading {
            "Updating on GPU…"
        } else {
            "↻ Refresh forecast"
        },
        sel!(refreshForecast:),
        207.,
        73.,
        151.,
        26.,
    );
    refresh.setEnabled(!loading && !demo);
    refresh.setKeyEquivalent(&NSString::from_str("r"));
    refresh.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
    let status = if loading {
        "Fetching model runs and buoy history · M3 GPU".into()
    } else if let Some(error) = error {
        error.to_string()
    } else if let Some(bundle) = bundle {
        format!(
            "Issued {} · {}{}",
            bundle
                .issued_at_utc
                .with_timezone(&Local)
                .format("%d %b %H:%M"),
            age_text(bundle.issued_at_utc),
            if bundle.is_stale() { " · delayed" } else { "" }
        )
    } else {
        "Refresh to generate your local GPU forecast".into()
    };
    let status_color = if error.is_some() || bundle.is_some_and(|b| b.is_stale()) {
        amber()
    } else {
        muted()
    };
    label(
        &view,
        mtm,
        &status,
        22.,
        109.,
        336.,
        18.,
        11.,
        0.,
        &status_color,
    );
    let readout = multiline(&view, mtm, "", 22., 150., 336., 45., 10., &ink());
    // Fixed-width digits keep the readout still while hovering across hours.
    readout.setFont(Some(&NSFont::monospacedDigitSystemFontOfSize_weight(
        10., 0.,
    )));
    let _ = view.ivars().readout.set(readout);
    view.update_readout();
    if let Some(buoy) = &view.ivars().buoy {
        for (channel, top, title) in [
            (0, 198., "SIGNIFICANT HEIGHT · Hs"),
            (1, 364., "PEAK PERIOD · Tp"),
        ] {
            label(&view, mtm, title, 22., top, 270., 16., 9., 0.5, &muted());
            let axis = |text: &str, y: f64| {
                label(&view, mtm, text, 4., y, 32., 16., 9., 0., &muted())
                    .setAlignment(NSTextAlignment::Right);
            };
            axis(&axis_text(scale_max(config, buoy, channel)), top + 19.);
            axis("0", top + 115.);
            for day in 0..7 {
                let index = (day * 24).min(buoy.points.len() - 1);
                let date = buoy.points[index]
                    .valid_at_utc
                    .with_timezone(&Local)
                    .format("%a %d")
                    .to_string();
                let x = chart_x(buoy, index) - 24.;
                label(&view, mtm, &date, x, top + 132., 48., 16., 9., 0., &muted())
                    .setAlignment(NSTextAlignment::Center);
            }
        }
        label(
            &view,
            mtm,
            "PEAK DIRECTION · ARROWS SHOW TRAVEL",
            22.,
            541.,
            336.,
            16.,
            9.,
            0.5,
            &muted(),
        );
        for day in 0..7 {
            let index = (day * 24).min(buoy.points.len() - 1);
            let point = compass(buoy.points[index].direction.from_deg);
            let x = chart_x(buoy, index) - 21.;
            label(&view, mtm, point, x, 601., 42., 16., 9., 0.4, &ink())
                .setAlignment(NSTextAlignment::Center);
        }
        multiline(
            &view,
            mtm,
            "Hover over the charts to inspect each hour.",
            22.,
            631.,
            336.,
            18.,
            10.,
            &muted(),
        );
        let note_value = if buoy.status == "experimental" {
            "Shading = nominal 80% range · direction = magnetic FROM\nExperimental: long-lead and winter coverage unverified."
        } else {
            "Shading = nominal 80% range · direction = magnetic FROM"
        };
        let note = multiline(&view, mtm, note_value, 22., 658., 336., 38., 10., &muted());
        if let Some(bundle) = bundle {
            note.setToolTip(Some(&NSString::from_str(&bundle.warnings.join("\n"))));
        }
        label(
            &view,
            mtm,
            "ECMWF WAM · GFS Wave",
            22.,
            709.,
            336.,
            18.,
            10.,
            0.,
            &muted(),
        );
    } else {
        multiline(
            &view,
            mtm,
            if demo {
                "Forecast refresh is disabled in demo mode."
            } else {
                "No saved forecast yet. Click Refresh forecast to fetch the latest model runs and generate height, period and direction on your GPU."
            },
            22.,
            235.,
            336.,
            100.,
            13.,
            &muted(),
        );
    }
    let tracking = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            view.bounds(),
            NSTrackingAreaOptions::MouseMoved
                | NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::ActiveInKeyWindow,
            Some(&view),
            None,
        )
    };
    view.addTrackingArea(&tracking);
    view.into_super()
}

#[cfg(test)]
mod tests {
    use super::axis_text;

    #[test]
    fn axis_label_keeps_a_decimal_for_small_scales() {
        assert_eq!(axis_text(1.296), "1.3");
        assert_eq!(axis_text(9.94), "9.9");
        assert_eq!(axis_text(20.6), "21");
    }
}
