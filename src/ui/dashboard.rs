use super::*;

pub(super) struct CanvasState {
    pub(super) snapshot: Snapshot,
    pub(super) page: Page,
    pub(super) config: Config,
    pub(super) charts: Vec<HistoryChart>,
    pub(super) hovered: Cell<Option<(usize, usize)>>,
    pub(super) hover_label: OnceCell<Retained<NSTextField>>,
}

define_class!(
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = CanvasState]
    pub(super) struct Dashboard;
    unsafe impl NSObjectProtocol for Dashboard {}
    impl Dashboard {
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
            if self.ivars().page != Page::Conditions { return; }
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
    pub(super) fn new(
        mtm: MainThreadMarker,
        snapshot: Snapshot,
        config: Config,
        forecast: Option<&ForecastBundle>,
    ) -> Retained<Self> {
        let view = Self::make(mtm, snapshot, config, Page::Conditions, forecast);
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
    pub(super) fn plain(mtm: MainThreadMarker, page: Page) -> Retained<Self> {
        Self::make(mtm, Snapshot::default(), Config::default(), page, None)
    }
    pub(super) fn make(
        mtm: MainThreadMarker,
        snapshot: Snapshot,
        config: Config,
        page: Page,
        forecast: Option<&ForecastBundle>,
    ) -> Retained<Self> {
        let predictions = forecast
            .map(|b| b.predictions_for(config.spot.id()))
            .unwrap_or_default();
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
    pub(super) fn hover_at(&self, location: NSPoint) {
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
    pub(super) fn set_hover(&self, selection: Option<(usize, usize)>) {
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
