use super::*;

pub(super) fn top_rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, HEIGHT - y - h), NSSize::new(w, h))
}
// Right edge of a single-line label's rendered text, for placing a trailing unit.
pub(super) fn text_end(label: &NSTextField) -> f64 {
    label.frame().origin.x + label.sizeThatFits(NSSize::new(10_000.0, 10_000.0)).width
}
pub(super) fn rgb(r: f64, g: f64, b: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0)
}
// Buoy observations and the forecast median.
pub(super) fn wave_color() -> Retained<NSColor> {
    rgb(0.03, 0.56, 0.64)
}
// Model predictions on the swell chart.
pub(super) fn model_color() -> Retained<NSColor> {
    rgb(0.57, 0.32, 0.79)
}
pub(super) fn ink() -> Retained<NSColor> {
    NSColor::labelColor()
}
pub(super) fn muted() -> Retained<NSColor> {
    NSColor::secondaryLabelColor()
}
pub(super) fn amber() -> Retained<NSColor> {
    NSColor::systemOrangeColor()
}
pub(super) fn fill_round(rect: NSRect, radius: f64, color: &NSColor) {
    color.setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius).fill();
}
pub(super) fn draw_arrow(x: f64, y: f64, from: f64, color: &NSColor) {
    draw_arrow_sized(x, y, from, 18.0, 3.0, color);
}
pub(super) fn draw_arrow_sized(
    x: f64,
    y: f64,
    from: f64,
    radius: f64,
    width: f64,
    color: &NSColor,
) {
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
pub(super) fn label(
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
pub(super) fn multiline(
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
pub(super) fn button(
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
pub(super) fn popup(
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
