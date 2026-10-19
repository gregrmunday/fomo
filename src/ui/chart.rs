use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum HistoryKind {
    Waves,
    Wind,
}

pub(super) struct ChartPoint {
    pub(super) sample_index: usize,
    pub(super) time: chrono::DateTime<chrono::Utc>,
    pub(super) position: NSPoint,
    pub(super) model_index: Option<usize>,
}

pub(super) struct HistoryChart {
    pub(super) kind: HistoryKind,
    pub(super) points: Vec<ChartPoint>,
    pub(super) plot: NSRect,
    pub(super) predictions: Vec<SwellPoint>,
    pub(super) start: chrono::DateTime<chrono::Utc>,
    pub(super) end: chrono::DateTime<chrono::Utc>,
    pub(super) low: f64,
    pub(super) high: f64,
}

impl HistoryChart {
    pub(super) fn new(snapshot: &Snapshot, kind: HistoryKind, predictions: &[SwellPoint]) -> Self {
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

    pub(super) fn position(&self, time: chrono::DateTime<chrono::Utc>, value: f64) -> NSPoint {
        NSPoint::new(
            self.plot.origin.x
                + (time - self.start).num_seconds() as f64
                    / (self.end - self.start).num_seconds() as f64
                    * self.plot.size.width,
            self.plot.origin.y
                + (value - self.low) / (self.high - self.low) * self.plot.size.height,
        )
    }

    pub(super) fn tracking_rect(&self) -> NSRect {
        NSRect::new(
            NSPoint::new(self.plot.origin.x - 4.0, self.plot.origin.y - 4.0),
            NSSize::new(self.plot.size.width + 8.0, self.plot.size.height + 8.0),
        )
    }

    pub(super) fn color(&self) -> Retained<NSColor> {
        match self.kind {
            HistoryKind::Waves => rgb(0.03, 0.56, 0.64),
            HistoryKind::Wind => NSColor::systemBlueColor(),
        }
    }

    pub(super) fn draw(&self) {
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

    pub(super) fn draw_dot(&self, point: NSPoint, radius: f64) {
        let dot = NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
            NSPoint::new(point.x - radius, point.y - radius),
            NSSize::new(radius * 2.0, radius * 2.0),
        ));
        self.color().setFill();
        dot.fill();
    }

    pub(super) fn bubble(&self, point: NSPoint) -> NSRect {
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
