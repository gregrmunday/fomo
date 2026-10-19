use crate::{forecast::*, model::*};
use chrono::Local;
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2_app_kit::*;
use objc2_foundation::{MainThreadMarker, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};
use std::cell::{Cell, OnceCell};

const HEIGHT: f64 = 744.0;
const LEFT: f64 = 40.0;
const WIDTH: f64 = 306.0;
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
                let value = |p: &ForecastPoint, quantile: usize| {
                    let q = if channel == 0 { &p.hs_m } else { &p.tp_s };
                    let raw = [q.p10, q.p50, q.p90][quantile];
                    if channel == 0 { self.ivars().config.height(raw) } else { raw }
                };
                let high = points.iter().map(|p| value(p, 2)).fold(0.0, f64::max).max(1.0) * 1.08;
                let pos = |i: usize, v: f64| NSPoint::new(LEFT+i as f64/(points.len()-1) as f64*WIDTH,
                    HEIGHT-top-height+v/high*height);
                for grid in 0..=2 {
                    let line = NSBezierPath::new();
                    line.moveToPoint(NSPoint::new(LEFT, HEIGHT-top-height+grid as f64/2.0*height));
                    line.lineToPoint(NSPoint::new(LEFT+WIDTH, HEIGHT-top-height+grid as f64/2.0*height));
                    NSColor::separatorColor().setStroke(); line.setLineWidth(0.5); line.stroke();
                }
                let band = NSBezierPath::new();
                band.moveToPoint(pos(0, value(&points[0], 0)));
                for (i,p) in points.iter().enumerate().skip(1) { band.lineToPoint(pos(i,value(p,0))); }
                for (i,p) in points.iter().enumerate().rev() { band.lineToPoint(pos(i,value(p,2))); }
                band.closePath();
                NSColor::colorWithCalibratedRed_green_blue_alpha(0.03,0.56,0.60,0.17).setFill(); band.fill();
                let path = NSBezierPath::new();
                for (i,p) in points.iter().enumerate() {
                    if i == 0 { path.moveToPoint(pos(i,value(p,1))); } else { path.lineToPoint(pos(i,value(p,1))); }
                }
                teal().setStroke(); path.setLineWidth(2.0); path.stroke();
                let index = self.ivars().selected.get().min(points.len()-1);
                let p = pos(index, value(&points[index],1));
                let guide = NSBezierPath::new();
                guide.moveToPoint(NSPoint::new(p.x,HEIGHT-top-height));
                guide.lineToPoint(NSPoint::new(p.x,HEIGHT-top));
                NSColor::tertiaryLabelColor().setStroke(); guide.setLineWidth(0.8); guide.stroke();
                teal().setFill(); NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::new(p.x-3.,p.y-3.),NSSize::new(6.,6.))).fill();
            }
            for day in 0..7 {
                let index = (day * 24).min(buoy.points.len()-1);
                let direction = buoy.points[index].direction.from_deg;
                let (dx,dy) = travel_vector(direction);
                let x = LEFT+index as f64/(buoy.points.len()-1) as f64*WIDTH;
                let y = HEIGHT-582.;
                let tip = NSPoint::new(x+dx*10.,y+dy*10.);
                let line = NSBezierPath::new();
                line.moveToPoint(NSPoint::new(x-dx*8.,y-dy*8.)); line.lineToPoint(tip);
                line.moveToPoint(NSPoint::new(tip.x-dx*5.-dy*3.,tip.y-dy*5.+dx*3.)); line.lineToPoint(tip);
                line.lineToPoint(NSPoint::new(tip.x-dx*5.+dy*3.,tip.y-dy*5.-dx*3.));
                teal().setStroke(); line.setLineWidth(1.8); line.stroke();
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
        let this = Self::alloc(mtm).set_ivars(ForecastIvars { buoy, config, selected: Cell::new(0), readout: OnceCell::new() });
        unsafe { msg_send![super(this), initWithFrame: NSRect::new(NSPoint::new(0.,0.),NSSize::new(380.,HEIGHT))] }
    }
    fn select_at(&self, point: NSPoint) {
        let Some(buoy) = &self.ivars().buoy else { return; };
        let from_top = HEIGHT-point.y;
        if !(200.0..=620.0).contains(&from_top) { return; }
        let index = (((point.x-LEFT)/WIDTH).clamp(0.,1.)*(buoy.points.len()-1) as f64).round() as usize;
        self.ivars().selected.set(index);
        self.update_readout();
        self.setNeedsDisplay(true);
    }
    fn update_readout(&self) {
        let Some(buoy) = &self.ivars().buoy else { return; };
        let p = &buoy.points[self.ivars().selected.get().min(buoy.points.len()-1)];
        let config = &self.ivars().config;
        let d = &p.direction;
        let text = format!("{} · {} {:.0}°\nHs {} ({:.1}–{:.1}{}) · Tp {:.1}s ({:.1}–{:.1}s)\nDirection range ±{:.0}°{}",
            p.valid_at_utc.with_timezone(&Local).format("%a %d %b, %H:%M"),compass(d.from_deg),d.from_deg,
            config.height_text(Some(p.hs_m.p50)),config.height(p.hs_m.p10),config.height(p.hs_m.p90),config.height_unit(),
            p.tp_s.p50,p.tp_s.p10,p.tp_s.p90,d.interval80.half_width_deg,
            if d.interval80.wraps_north { " · crosses north" } else if d.interval80.full_circle { " · all directions" } else { "" });
        if let Some(label) = self.ivars().readout.get() { label.setStringValue(&NSString::from_str(&text)); }
    }
}
fn rect(x:f64,y:f64,w:f64,h:f64)->NSRect { NSRect::new(NSPoint::new(x,HEIGHT-y-h),NSSize::new(w,h)) }
fn teal()->Retained<NSColor> { NSColor::colorWithCalibratedRed_green_blue_alpha(0.03,0.56,0.60,1.0) }
fn text(parent:&NSView,mtm:MainThreadMarker,value:&str,x:f64,y:f64,w:f64,h:f64,size:f64)->Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(value),mtm);
    label.setFrame(rect(x,y,w,h)); label.setFont(Some(&NSFont::systemFontOfSize(size)));
    label.setTextColor(Some(&NSColor::labelColor())); parent.addSubview(&label); label
}
fn button(parent:&NSView,mtm:MainThreadMarker,target:&AnyObject,title:&str,action:Sel,x:f64,y:f64,w:f64)->Retained<NSButton> {
    let b = unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title),Some(target),Some(action),mtm) };
    b.setFrame(rect(x,y,w,28.)); b.setBezelStyle(NSBezelStyle::Push); b.setBordered(false);
    b.setFont(Some(&NSFont::systemFontOfSize(11.))); parent.addSubview(&b); b
}
pub fn view(mtm:MainThreadMarker,target:&AnyObject,config:&Config,bundle:Option<&ForecastBundle>,loading:bool,error:Option<&str>,demo:bool)->Retained<NSView> {
    let buoy = bundle.and_then(|b| b.for_station(config.spot.id())).cloned();
    let view = ForecastCanvas::new(mtm,buoy,config.clone());
    text(&view,mtm,"F O M O   ·   FORECAST",22.,19.,336.,26.,14.);
    let name=view.ivars().buoy.as_ref().map(|b|b.name.as_str()).unwrap_or(config.spot.name());
    text(&view,mtm,&format!("{name} · next seven days"),22.,49.,336.,25.,20.);
    button(&view,mtm,target,"‹ Observations",sel!(back:),17.,85.,112.);
    let refresh = button(&view,mtm,target,if loading { "Updating on GPU…" } else { "↻ Refresh forecast" },sel!(refreshForecast:),207.,85.,151.);
    refresh.setEnabled(!loading && !demo);
    refresh.setKeyEquivalent(&NSString::from_str("r")); refresh.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
    let status = if loading { "Fetching model runs and buoy history · M3 GPU".into() }
        else if let Some(error)=error { format!("Update failed: {error}") }
        else if let Some(bundle)=bundle { format!("Issued {} · {}{}",bundle.issued_at_utc.with_timezone(&Local).format("%d %b %H:%M"),age_text(bundle.issued_at_utc),if bundle.is_stale(){" · delayed"}else{""}) }
        else { "Refresh to generate your local GPU forecast".into() };
    let status_label=text(&view,mtm,&status,22.,121.,336.,24.,10.);
    status_label.setToolTip(Some(&NSString::from_str(&status)));
    if error.is_some() || bundle.is_some_and(|b|b.is_stale()) { status_label.setTextColor(Some(&NSColor::systemOrangeColor())); }
    let readout=text(&view,mtm,"",22.,150.,336.,45.,10.);
    let _=view.ivars().readout.set(readout); view.update_readout();
    if let Some(buoy)=&view.ivars().buoy {
        for (channel,top,title) in [(0,198.,"SIGNIFICANT HEIGHT Hs"),(1,364.,"PEAK PERIOD Tp")] {
            text(&view,mtm,title,22.,top,270.,18.,10.);
            let high=buoy.points.iter().map(|p|if channel==0 {config.height(p.hs_m.p90)}else{p.tp_s.p90}).fold(0.,f64::max).max(1.)*1.08;
            text(&view,mtm,&format!("{high:.0}"),12.,top+19.,27.,16.,9.);
            text(&view,mtm,"0",24.,top+115.,15.,16.,9.);
            for day in 0..7 {
                let index=(day*24).min(buoy.points.len()-1);
                let x=LEFT+index as f64/(buoy.points.len()-1)as f64*WIDTH-17.;
                text(&view,mtm,&buoy.points[index].valid_at_utc.with_timezone(&Local).format("%a %d").to_string(),x,top+132.,48.,16.,8.);
            }
        }
        text(&view,mtm,"PEAK DIRECTION · ARROWS SHOW TRAVEL",22.,541.,336.,18.,10.);
        for day in 0..7 {
            let index=(day*24).min(buoy.points.len()-1);
            let x=LEFT+index as f64/(buoy.points.len()-1)as f64*WIDTH-13.;
            text(&view,mtm,compass(buoy.points[index].direction.from_deg),x,601.,42.,16.,9.);
        }
        text(&view,mtm,"Hover over the charts to inspect each hour.",22.,631.,336.,18.,10.);
        let note_value=if buoy.status=="experimental" {
            "Shading = nominal 80% range · direction = magnetic FROM\nExperimental: long-lead and winter coverage unverified."
        } else { "Shading = nominal 80% range · direction = magnetic FROM" };
        let note=text(&view,mtm,note_value,22.,658.,336.,38.,10.);
        if let Some(bundle)=bundle { note.setToolTip(Some(&NSString::from_str(&bundle.warnings.join("\n")))); }
        text(&view,mtm,"ECMWF WAM · GFS Wave",22.,709.,336.,18.,10.);
    } else {
        text(&view,mtm,if demo {"Forecast refresh is disabled in demo mode."}else{"No saved forecast yet. Click Refresh forecast to fetch the latest model runs and generate height, period and direction on your GPU."},22.,235.,336.,100.,14.);
    }
    let tracking=unsafe { NSTrackingArea::initWithRect_options_owner_userInfo(NSTrackingArea::alloc(),view.bounds(),
        NSTrackingAreaOptions::MouseMoved|NSTrackingAreaOptions::MouseEnteredAndExited|NSTrackingAreaOptions::ActiveInKeyWindow,Some(&view),None) };
    view.addTrackingArea(&tracking);
    view.into_super()
}
