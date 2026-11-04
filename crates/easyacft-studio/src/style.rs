//! Diagram colours and shapes: dark slate panel, green supplied, grey off, orange degraded, red failed, squircle boxes.

use eframe::egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

pub const BG: Color32 = Color32::from_rgb(27, 29, 32);
pub const PANEL: Color32 = Color32::from_rgb(37, 39, 43);
pub const FILL: Color32 = Color32::from_rgb(31, 34, 38);
pub const GREEN: Color32 = Color32::from_rgb(63, 210, 127);
pub const ORANGE: Color32 = Color32::from_rgb(255, 176, 0);
pub const RED: Color32 = Color32::from_rgb(255, 61, 99);
pub const GREY: Color32 = Color32::from_rgb(97, 101, 108);
pub const GREY_TEXT: Color32 = Color32::from_rgb(139, 144, 151);
pub const WARN_FILL: Color32 = Color32::from_rgb(58, 47, 20);
pub const SELECT: Color32 = Color32::from_rgb(255, 77, 106);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum State {
    #[default]
    Off,
    Live,
    Warn,
    Fail,
}

pub fn colour_of(s: State) -> Color32 {
    match s {
        State::Off => GREY,
        State::Live => GREEN,
        State::Warn => ORANGE,
        State::Fail => RED,
    }
}

pub fn text_of(s: State) -> Color32 {
    match s {
        State::Off => GREY_TEXT,
        State::Fail => Color32::WHITE,
        s => colour_of(s),
    }
}

pub fn stroke_of(s: State, scale: f32) -> Stroke {
    Stroke::new(if s == State::Off { 1.4 } else { 2.4 } * scale.max(0.6), colour_of(s))
}

pub fn fill_of(s: State) -> Color32 {
    match s {
        State::Fail => RED,
        State::Warn => WARN_FILL,
        _ => FILL,
    }
}

/// Continuous-curvature rounded box.
pub fn squircle_points(r: Rect, radius: f32) -> Vec<Pos2> {
    let c = (radius * 1.5).min(r.width() / 2.0).min(r.height() / 2.0);
    let p = 2.0 / 4.5;
    let f = |v: f32| v.max(0.0).powf(p);
    let mut pts = Vec::with_capacity(44);
    let mut arc = |cx: f32, cy: f32, sx: f32, sy: f32, from: f32, to: f32| {
        for i in 0..=10 {
            let t = from + (to - from) * i as f32 / 10.0;
            pts.push(Pos2::new(cx + sx * c * f(t.cos()), cy + sy * c * f(t.sin())));
        }
    };
    let q = std::f32::consts::FRAC_PI_2;
    arc(r.right() - c, r.top() + c, 1.0, -1.0, q, 0.0);
    arc(r.right() - c, r.bottom() - c, 1.0, 1.0, 0.0, q);
    arc(r.left() + c, r.bottom() - c, -1.0, 1.0, q, 0.0);
    arc(r.left() + c, r.top() + c, -1.0, -1.0, 0.0, q);
    pts
}

pub fn squircle(p: &Painter, r: Rect, radius: f32, s: State) {
    let scale = (r.height() / 34.0).clamp(0.5, 1.5);
    p.add(Shape::convex_polygon(squircle_points(r, radius), fill_of(s), stroke_of(s, scale)));
}

pub fn squircle_with(p: &Painter, r: Rect, radius: f32, fill: Color32, stroke: Stroke) {
    p.add(Shape::convex_polygon(squircle_points(r, radius), fill, stroke));
}

/// Diagram label: part ids read as panel names, `ac_ess` as `AC ESS`.
pub fn label(id: &str) -> String {
    id.replace('_', " ").to_uppercase()
}
