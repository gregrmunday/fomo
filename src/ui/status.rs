use super::*;

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Option<f64>]
    pub(super) struct BearingArrow;
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
    pub(super) fn new(mtm: MainThreadMarker, direction: Option<f64>, size: f64) -> Retained<Self> {
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
    pub(super) struct StatusReadings;
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
    pub(super) fn new(mtm: MainThreadMarker, state: &State, height: f64) -> Retained<Self> {
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

pub(super) fn direction_row(
    mtm: MainThreadMarker,
    prefix: &str,
    direction: Option<f64>,
) -> Retained<NSView> {
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

pub(super) fn wave_icon() -> Option<Retained<NSImage>> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str("water.waves"),
        Some(&NSString::from_str("Fomo surf conditions")),
    )?;
    image.setTemplate(true);
    image.setSize(NSSize::new(17.0, 16.0));
    Some(image)
}

pub(super) fn status_label(
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
