//! EasyAcft Studio: open an aircraft folder, watch every system run, edit its files.

mod diagram;
mod cutaway;
mod edit;
mod engine_view;
mod inspector;
mod live;
mod png;
mod style;
mod svg;
mod why;

use diagram::{Diagram, Role, DOMAINS};
use style::State;
use easyacft_lang::{load, Model, Type};
use easyacft_parts::catalog;
use easyacft_parts::Aircraft;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Diagram,
    Engine,
    Plot,
    Log,
    Health,
    Tables,
    Fit,
    Compare,
    Computers,
}

pub struct App {
    pub dir: String,
    pub aircraft: Option<Aircraft>,
    pub model: Option<Model>,
    report: String,
    running: bool,
    speed: f64,
    tab: Tab,
    domain: usize,
    pub selected: Option<usize>,
    search: String,
    pub plots: Vec<(String, Vec<[f64; 2]>)>,
    pinned: HashMap<String, (f32, f32)>,
    fold: bool,
    full: bool,
    toggle_full: bool,
    saved_ppp: Option<f32>,
    dragging: Option<usize>,
    offset: Vec2,
    pub edits: HashMap<String, String>,
    live: Option<live::Client>,
    pub status: String,
    add_kind: String,
    add_id: String,
    add_file: usize,
    tables: Vec<(PathBuf, Vec<Vec<String>>)>,
    fit_points: String,
    fit_tune: String,
    fit_out: String,
    compare_dir: String,
    compare_out: Vec<String>,
    help: bool,
    last: Instant,
    shot: Option<(String, u32)>,
    zoom: f32,
    fit: bool,
    pub history: edit::History,
    pub all_settings: bool,
    pub bind_out: String,
    pub bind_ref: String,
    pub bind_unit: String,
    pub bind_write: bool,
    trial: Option<easyacft_core::Snapshot>,
    timeline: std::collections::VecDeque<(f64, easyacft_core::Snapshot)>,
    scrub: Option<usize>,
    recording: Option<Vec<(f64, String, f64)>>,
    record_start: f64,
    connect_from: Option<usize>,
    theme: Theme,
    template: usize,
    new_dir: String,
    export_dir: String,
    show_changes: bool,
    zoomed: bool,
    fit_size: Vec2,
    engine_view: Option<String>,
    engine_pick: Option<String>,
    cascade: Option<Cascade>,
    readers: Option<Vec<Vec<usize>>>,
    engine_show: u8,
    steady: HashMap<String, (easyacft_parts::engine::Gas, [f64; 3])>,
}

/// Every on/off value as it was when a failure went in, and when each one first changed after it.
struct Cascade {
    cause: String,
    start: f64,
    base: Vec<(usize, f64)>,
    seen: Vec<bool>,
    /// Slot, seconds after the failure, value before.
    first: Vec<(usize, f64, f64)>,
}

#[derive(Clone, Copy)]
struct Theme {
    ink: Color32,
    faint: Color32,
    fill: Color32,
    paper: Color32,
}

fn hex(s: &str) -> Option<Color32> {
    let h = s.trim().trim_matches('"').trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).ok()?;
    (h.len() == 6).then(|| Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8))
}

/// Colours from `studio-theme.toml` next to the aircraft (ink, faint, fill, paper as "#rrggbb").
fn read_theme(dir: &str) -> Theme {
    let mut t = Theme { ink: Color32::from_gray(20), faint: Color32::from_gray(150), fill: Color32::from_gray(248), paper: Color32::from_gray(248) };
    let text = std::fs::read_to_string(Path::new(dir).join("studio-theme.toml")).unwrap_or_default();
    for l in text.lines() {
        let Some((k, v)) = l.split_once('=') else { continue };
        let Some(c) = hex(v) else { continue };
        match k.trim() {
            "ink" => t.ink = c,
            "faint" => t.faint = c,
            "fill" => t.fill = c,
            "paper" => t.paper = c,
            _ => {}
        }
    }
    t
}

/// Aircraft folders offered as templates for a new aircraft.
fn templates() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(e) = std::env::current_exe() {
        for a in e.ancestors().skip(1).take(4) {
            roots.push(a.join("examples"));
        }
    }
    roots.push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples"));
    for r in roots {
        if let Ok(rd) = std::fs::read_dir(&r) {
            let mut v: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.join("aircraft.toml").exists()).collect();
            if !v.is_empty() {
                v.sort();
                return v;
            }
        }
    }
    Vec::new()
}

fn last_file() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("EasyAcft").join("last-aircraft.txt"))
}

fn last_dir() -> String {
    last_file().and_then(|f| std::fs::read_to_string(f).ok()).map(|s| s.trim().to_owned()).unwrap_or_default()
}

fn remember_dir(dir: &str) {
    if let Some(f) = last_file() {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(f, dir);
    }
}

fn layout_file(dir: &str) -> PathBuf {
    Path::new(dir).join("studio-layout.toml")
}

fn read_layout(dir: &str) -> HashMap<String, (f32, f32)> {
    let text = std::fs::read_to_string(layout_file(dir)).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            let v = v.trim().trim_start_matches('[').trim_end_matches(']');
            let (x, y) = v.split_once(',')?;
            Some((k.trim().trim_matches('"').to_owned(), (x.trim().parse().ok()?, y.trim().parse().ok()?)))
        })
        .collect()
}

fn write_layout(dir: &str, pinned: &HashMap<String, (f32, f32)>) {
    let mut keys: Vec<&String> = pinned.keys().collect();
    keys.sort();
    let body: String = keys.iter().map(|k| format!("\"{k}\" = [{:.0}, {:.0}]\n", pinned[*k].0, pinned[*k].1)).collect();
    let _ = std::fs::write(layout_file(dir), format!("[pos]\n{body}"));
}

pub fn unit(t: Type, v: f64) -> String {
    use easyacft_core::Dim;
    match t {
        Type::Bool => (if v != 0.0 { "yes" } else { "no" }).to_owned(),
        Type::Num(d) if d == Dim::VOLTAGE => format!("{v:.1} V"),
        Type::Num(d) if d == Dim::CURRENT => format!("{v:.1} A"),
        Type::Num(d) if d == Dim::POWER => format!("{:.2} kW", v / 1e3),
        Type::Num(d) if d == Dim::PRESSURE => format!("{:.0} psi", v / 6894.757),
        Type::Num(d) if d == Dim::TEMPERATURE => format!("{:.0} °C", v - 273.15),
        Type::Num(d) if d == Dim::MASS => format!("{v:.0} kg"),
        Type::Num(d) if d == Dim::MASS_FLOW => format!("{v:.3} kg/s"),
        Type::Num(d) if d == Dim::FORCE => format!("{:.1} kN", v / 1e3),
        Type::Num(d) if d == Dim::FREQUENCY => format!("{v:.1} Hz"),
        Type::Num(d) if d == Dim::LENGTH => format!("{v:.1} m"),
        Type::Num(d) if d == Dim::SPEED => format!("{v:.1} m/s"),
        Type::Num(d) if d == Dim::VOLUME => format!("{:.1} L", v * 1e3),
        Type::Num(d) if d == Dim::VOLUME_FLOW => format!("{:.1} L/min", v * 6e4),
        Type::Num(d) if d == Dim::TIME => format!("{v:.1} s"),
        Type::Num(_) => format!("{v:.3}"),
    }
}

pub const SETTABLE: &[&str] = &["on", "pressed", "guard_open", "failed", "pulled", "frozen", "bad_data", "channel_failed", "position", "value", "mass"];
const MORE: &[&str] = &["custom", "logic", "weight", "gear", "cabin", "ice", "oxygen", "fadec", "apu", "fire", "light", "sound", "failure", "data", "sensor", "wear", "input", "output", "environment"];

impl App {
    fn new(dir: String) -> Self {
        let mut a = App {
            dir,
            aircraft: None,
            model: None,
            report: String::new(),
            running: false,
            speed: 1.0,
            tab: Tab::Diagram,
            domain: 0,
            selected: None,
            search: String::new(),
            plots: Vec::new(),
            pinned: HashMap::new(),
            fold: false,
            full: false,
            toggle_full: false,
            saved_ppp: None,
            dragging: None,
            offset: Vec2::ZERO,
            edits: HashMap::new(),
            live: None,
            status: String::new(),
            add_kind: "electrical.bus".into(),
            add_id: String::new(),
            add_file: 0,
            tables: Vec::new(),
            fit_points: "[[point]]\nthrust = 1.0\nfuel_flow = \"1.0 kg/s\"\n".into(),
            fit_tune: "hpc_efficiency,hpt_efficiency".into(),
            fit_out: String::new(),
            compare_dir: String::new(),
            compare_out: Vec::new(),
            help: false,
            last: Instant::now(),
            shot: None,
            zoom: 1.0,
            fit: true,
            history: edit::History::default(),
            all_settings: false,
            bind_out: String::new(),
            bind_ref: String::new(),
            bind_unit: String::new(),
            bind_write: false,
            trial: None,
            timeline: Default::default(),
            scrub: None,
            recording: None,
            record_start: 0.0,
            connect_from: None,
            theme: read_theme(""),
            template: 0,
            new_dir: String::new(),
            export_dir: String::new(),
            show_changes: false,
            zoomed: false,
            fit_size: Vec2::ZERO,
            engine_view: None,
            engine_pick: None,
            cascade: None,
            readers: None,
            engine_show: 0,
            steady: HashMap::new(),
        };
        if !a.dir.is_empty() {
            a.reload();
        }
        a
    }

    fn reload(&mut self) {
        let path = PathBuf::from(&self.dir);
        let (model, _) = load(&path);
        self.model = Some(model);
        self.pinned = read_layout(&self.dir);
        match Aircraft::load(&path) {
            Ok((mut fresh, report)) => {
                if let Some(old) = &self.aircraft {
                    for (i, (n, _)) in old.checked.slots.names.iter().enumerate() {
                        fresh.set(n, old.slots[i]);
                    }
                    fresh.time = old.time;
                }
                self.report = report.render();
                self.status = format!("{}: {} parts", fresh.name, fresh.checked.parts.len());
                if self.shot.is_none() {
                    remember_dir(&self.dir);
                }
                self.aircraft = Some(fresh);
            }
            Err(report) => {
                self.report = report.render();
                self.status = format!("{} errors; the last good version keeps running", report.errors());
            }
        }
        self.tables = scan_tables(&path);
        self.readers = None;
        self.steady.clear();
        self.theme = read_theme(&self.dir);
        if let Some(of) = std::fs::read_to_string(path.join("aircraft.toml")).ok().and_then(|t| t.parse::<toml_edit::DocumentMut>().ok()).and_then(|d| d.get("variant").and_then(|v| v.get("of")).and_then(|v| v.as_str()).map(str::to_owned)) {
            self.compare_dir = path.join(of).display().to_string();
        }
        self.edits.clear();
    }

    fn value(&self, slot: usize) -> f64 {
        match (&self.live, &self.aircraft) {
            (Some(l), Some(a)) if l.matches(a) => l.values.get(slot).copied().unwrap_or(0.0),
            (_, Some(a)) => a.slots.get(slot).copied().unwrap_or(0.0),
            _ => 0.0,
        }
    }

    pub fn get(&self, name: &str) -> Option<f64> {
        let a = self.aircraft.as_ref()?;
        a.slot(name).map(|s| self.value(s))
    }

    pub fn set(&mut self, name: &str, v: f64) {
        if let (Some(r), Some(a)) = (&mut self.recording, &self.aircraft) {
            r.push((a.time - self.record_start, name.to_owned(), v));
        }
        if let Some(l) = &mut self.live {
            l.send(name, v);
        }
        if let Some(a) = &mut self.aircraft {
            a.set(name, v);
        }
    }

    /// Sets a failure value and starts recording what changes because of it.
    pub fn inject(&mut self, path: &str, v: f64, cause: String) {
        if let Some(a) = &self.aircraft {
            let base: Vec<(usize, f64)> = a.checked.slots.names.iter().enumerate().filter(|(_, (_, t))| matches!(t, Type::Bool)).map(|(i, _)| (i, self.value(i))).collect();
            let seen = vec![false; base.len()];
            self.cascade = Some(Cascade { cause, start: a.time, base, seen, first: Vec::new() });
        }
        self.set(path, v);
    }

    fn note_cascade(&mut self) {
        let Some(t) = self.aircraft.as_ref().map(|a| a.time) else { return };
        let Some(c) = &self.cascade else { return };
        let fresh: Vec<usize> = (0..c.base.len()).filter(|&k| !c.seen[k] && self.value(c.base[k].0) != c.base[k].1).collect();
        if let Some(c) = &mut self.cascade {
            for k in fresh {
                c.seen[k] = true;
                c.first.push((c.base[k].0, t - c.start, c.base[k].1));
            }
        }
    }

    /// For each part, the parts whose expressions read it or that reference it.
    fn readers(&mut self) -> &Vec<Vec<usize>> {
        if self.readers.is_none() {
            let mut r: Vec<Vec<usize>> = Vec::new();
            if let Some(a) = &self.aircraft {
                let c = &a.checked;
                r = vec![Vec::new(); c.parts.len()];
                for (j, p) in c.parts.iter().enumerate() {
                    let mut from: Vec<usize> = Vec::new();
                    for (_, src) in &p.exprs {
                        for tok in src.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')) {
                            if let Some((id, _)) = tok.split_once('.') {
                                if let Some(i) = c.find(id) {
                                    from.push(i);
                                }
                            }
                        }
                    }
                    for (_, v) in &p.values {
                        match v {
                            easyacft_lang::Value::Ref(i) => from.push(*i),
                            easyacft_lang::Value::Refs(is) => from.extend(is.iter().copied()),
                            _ => {}
                        }
                    }
                    from.sort_unstable();
                    from.dedup();
                    for i in from.into_iter().filter(|&i| i != j) {
                        r[i].push(j);
                    }
                }
            }
            self.readers = Some(r);
        }
        self.readers.as_ref().unwrap()
    }

    fn tick(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f64().min(0.25);
        self.last = now;
        if let Some(l) = &mut self.live {
            l.poll();
        }
        let live = self.live.as_ref().is_some_and(|l| l.connected);
        if self.running && !live && self.scrub.is_none() {
            if let Some(a) = &mut self.aircraft {
                let mut left = dt * self.speed;
                while left > 1e-9 {
                    let s = left.min(0.05);
                    a.step(s);
                    left -= s;
                }
            }
        }
        if let Some(a) = &self.aircraft {
            if self.scrub.is_none() && self.timeline.back().is_none_or(|(t, _)| a.time - t >= 1.0 || a.time < *t) {
                if self.timeline.back().is_some_and(|(t, _)| a.time < *t) {
                    self.timeline.clear();
                }
                self.timeline.push_back((a.time, a.snapshot()));
                while self.timeline.len() > 300 {
                    self.timeline.pop_front();
                }
            }
        }
        let t = self.aircraft.as_ref().map_or(0.0, |a| a.time);
        let names: Vec<String> = self.plots.iter().map(|p| p.0.clone()).collect();
        for (k, n) in names.iter().enumerate() {
            if let Some(v) = self.get(n) {
                let pts = &mut self.plots[k].1;
                if pts.last().is_none_or(|p| t - p[0] > 0.05) {
                    pts.push([t, v]);
                }
                pts.retain(|p| t - p[0] < 300.0);
            }
        }
        self.note_cascade();
    }

    fn part_state(&self, p: &easyacft_lang::CheckedPart) -> (bool, bool, String) {
        diagram::state(p, |n| self.get(n))
    }

    fn draw_diagram(&mut self, ui: &mut egui::Ui) {
        let Some(a) = &self.aircraft else {
            ui.label("Open an aircraft folder: the one with aircraft.toml.");
            return;
        };
        let domain = DOMAINS[self.domain];
        let avail = ui.available_size();
        let d: Diagram = diagram::build(&a.checked, domain, &self.pinned, self.fold, (avail.x / avail.y.max(1.0)).clamp(0.6, 3.0));
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        if !self.zoomed && self.fit_size != resp.rect.size() {
            self.fit = true;
        }
        if self.fit {
            self.fit_size = resp.rect.size();
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            let pts = d.items.iter().flat_map(|it| [(it.pos.0 - it.size.0 / 2.0, it.pos.1 - it.size.1 / 2.0 - 16.0), (it.pos.0 + it.size.0 / 2.0, it.pos.1 + it.size.1 / 2.0 + 16.0)]).chain(d.edges.iter().flat_map(|e| e.path.iter().copied())).chain(d.panels.iter().flat_map(|p| [p.pos, (p.pos.0 + p.size.0, p.pos.1 + p.size.1)])).chain(d.rows.iter().flat_map(|r| [(r.pos.0, r.pos.1 - diagram::ROW_H / 2.0), (r.pos.0 + r.w, r.pos.1 + diagram::ROW_H / 2.0)]));
            for (px, py) in pts {
                (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
            }
            if x0 <= x1 {
                let pad = 20.0;
                self.zoom = ((resp.rect.width() - 2.0 * pad) / (x1 - x0)).min((resp.rect.height() - 2.0 * pad) / (y1 - y0)).clamp(0.02, 1.5);
                self.offset = Vec2::new(pad - x0 * self.zoom, pad - y0 * self.zoom);
            }
            self.fit = false;
        }
        if let Some(h) = resp.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old = self.zoom;
                self.zoom = (self.zoom * (1.0 + scroll * 0.002)).clamp(0.02, 3.0);
                self.zoomed = true;
                let anchor = h - resp.rect.min - self.offset;
                self.offset -= anchor * (self.zoom / old - 1.0);
            }
        }
        let z = self.zoom;
        let origin = resp.rect.min.to_vec2() + self.offset;
        let (ink, faint) = (style::GREEN, style::GREY_TEXT);
        painter.rect_filled(resp.rect, 0.0, style::BG);
        painter.rect_filled(resp.rect.shrink(6.0), 10.0, style::PANEL);
        let labels = z >= 0.45;
        let small = FontId::monospace(10.0 * z);
        let fit = |text: &str, size: f32, max_w: f32, colour: Color32| -> Option<std::sync::Arc<egui::Galley>> {
            let g = painter.layout_no_wrap(text.to_owned(), FontId::monospace(size), colour);
            let k = (max_w / g.size().x.max(1.0)).min(1.0);
            (size * k >= 6.0).then(|| if k < 1.0 { painter.layout_no_wrap(text.to_owned(), FontId::monospace(size * k), colour) } else { g })
        };
        let at = |p: (f32, f32)| Pos2::new(p.0 * z, p.1 * z) + origin;
        let box_of = |it: &diagram::Item| Rect::from_center_size(at(it.pos), Vec2::new(it.size.0, it.size.1) * z);
        let mut occupied: Vec<Rect> = d.items.iter().map(|it| box_of(it).expand(2.0)).collect();
        let state_of = |live: bool, failed: bool, text: &str| if failed { State::Fail } else if text.contains("tripped") || text.contains("shed") { State::Warn } else if live { State::Live } else { State::Off };
        let wire = |pts: &[Pos2], st: State| {
            painter.add(egui::Shape::line(pts.to_vec(), Stroke::new(if st == State::Off { 1.4 * z.max(0.6) } else { 2.6 * z.max(0.6) }, style::colour_of(st))));
        };
        for (part, l) in &d.links {
            let (live, failed, text) = self.part_state(&a.checked.parts[*part]);
            wire(&[at(l[0]), at(l[1])], state_of(live, failed, &text));
        }
        let mut hit = None;
        let mut edge_labels = Vec::new();
        for e in &d.edges {
            let (live, failed, text) = self.part_state(&a.checked.parts[e.part]);
            let st = state_of(live, failed, &text);
            let pts: Vec<Pos2> = e.path.iter().map(|&p| at(p)).collect();
            wire(&pts, if live { State::Live } else if failed { State::Fail } else { State::Off });
            let c = at(e.mark);
            let rad = 9.0 * z;
            let sel = self.selected == Some(e.part);
            painter.circle(c, rad, if st == State::Fail { style::RED } else { style::FILL }, Stroke::new(2.0f32 * z.max(0.6), if sel { style::SELECT } else { style::colour_of(st) }));
            let along = if e.vertical { Vec2::new(0.0, 1.0) } else { Vec2::new(1.0, 0.0) };
            let dir = if live { along } else { Vec2::new(along.y, along.x) };
            painter.line_segment([c - dir * rad * 0.75, c + dir * rad * 0.75], Stroke::new(2.2f32 * z.max(0.6), if st == State::Fail { Color32::WHITE } else { style::colour_of(st) }));
            occupied.push(Rect::from_center_size(c, Vec2::splat(2.0 * rad + 2.0)));
            if labels {
                let t = if text.is_empty() { style::label(&a.checked.parts[e.part].id) } else { format!("{} {text}", style::label(&a.checked.parts[e.part].id)) };
                edge_labels.push((c, rad, e.vertical, t, st));
            }
            if resp.hover_pos().is_some_and(|h| (h - c).length() < rad + 5.0) {
                hit = Some(e.part);
            }
        }
        for p in &d.panels {
            let r = Rect::from_min_size(at(p.pos), Vec2::new(p.size.0, p.size.1) * z);
            painter.rect_filled(r, 8.0 * z, style::FILL);
            painter.rect_stroke(r, 8.0 * z, Stroke::new(1.0f32, style::GREY_TEXT));
            if let Some(g) = fit(&style::label(&a.checked.parts[p.hub].id), (13.0 * z).max(11.0), r.width() - 16.0 * z, ink) {
                painter.galley(Pos2::new(r.left() + 10.0 * z, r.top() + 10.0 * z), g, ink);
            }
        }
        for row in &d.rows {
            let left = at((row.pos.0, row.pos.1));
            let (live, failed, text) = row.feeds.first().map(|&f| self.part_state(&a.checked.parts[f])).unwrap_or_else(|| self.part_state(&a.checked.parts[row.bus]));
            let st = state_of(live, failed, &text);
            let rad = 5.0 * z;
            painter.circle(left + Vec2::new(rad, 0.0), rad, if st == State::Fail { style::RED } else { style::FILL }, Stroke::new(1.5f32 * z.max(0.6), style::colour_of(st)));
            let names: Vec<String> = row.feeds.iter().map(|&f| style::label(&a.checked.parts[f].id).trim_start_matches("CB ").to_owned()).collect();
            let feed = row.feeds.iter().zip(&names).map(|(&f, n)| if a.checked.parts[f].kind == "contactor" || a.checked.parts[f].kind == "tru" { n.clone() } else { format!("{n} {:.0}A", a.checked.parts[f].num("rating")) }).collect::<Vec<_>>().join(" + ");
            let users = row.users.iter().map(|&u| {
                let (_, _, t) = self.part_state(&a.checked.parts[u]);
                let n = style::label(&a.checked.parts[u].id);
                let base = n.trim_end_matches(" LOAD");
                // A load named after its own breaker only needs its reading.
                if names.iter().any(|f| f.trim_end_matches(" NORMAL BKR").trim_end_matches(" 2ND BKR") == base) { t } else { format!("{base} {t}") }
            }).collect::<Vec<_>>().join(", ");
            let line = match row.to {
                Some(b) => format!("{feed}  tie to {}", style::label(&a.checked.parts[b].id)),
                None if feed.is_empty() => users,
                None => format!("{feed}   {users}"),
            };
            if let Some(g) = fit(&line, 10.0 * z, (row.w - 16.0) * z, style::text_of(st)) {
                painter.galley(Pos2::new(left.x + 2.0 * rad + 5.0 * z, left.y - g.size().y / 2.0), g, style::text_of(st));
            }
            let hit_r = Rect::from_min_size(Pos2::new(left.x, left.y - diagram::ROW_H * z / 2.0), Vec2::new(row.w * z, diagram::ROW_H * z));
            if resp.hover_pos().is_some_and(|h| hit_r.contains(h)) {
                hit = row.feeds.first().copied().or_else(|| row.users.first().copied());
            }
        }
        // Names too big for their box are placed afterwards, map-style: the most important first, only where they hit nothing.
        let mut pending: Vec<(u32, Pos2, String, Color32, Rect)> = Vec::new();
        for it in &d.items {
            let p = at(it.pos);
            let part = &a.checked.parts[it.part];
            let (live, failed, text) = self.part_state(part);
            let st = state_of(live, failed, &text);
            let sel = self.selected == Some(it.part);
            let name = style::label(&part.id);
            let rect = box_of(it);
            style::squircle(&painter, rect, 9.0 * z, st);
            if sel {
                style::squircle_with(&painter, rect.expand(3.0 * z), 11.0 * z, Color32::TRANSPARENT, Stroke::new(2.0f32, style::SELECT));
            }
            let size = if it.role == Role::Hub { 12.5 * z } else { 10.0 * z };
            let inner = rect.width() - 10.0 * z;
            let mut sub: Vec<String> = Vec::new();
            if labels && !text.is_empty() {
                sub.push(text.clone());
            }
            if let Some(&(b, u)) = d.folded.get(&it.part) {
                sub.push(if b == 0 { format!("{u} USERS") } else { format!("{b} CB  {u} USERS") });
            }
            let sub_g = if sub.is_empty() { None } else { fit(&sub.join("  "), 9.0 * z, inner, faint) };
            let colour = style::text_of(st);
            let mut short = name.trim_end_matches(" LOAD").trim_end_matches(" BUS").to_owned();
            // A load named after the bus it hangs on shows its reading; the bus box already carries the name.
            let own_bus = part.part("bus").map(|b| style::label(&a.checked.parts[b].id).trim_end_matches(" BUS").to_owned());
            if it.role == Role::Sink && own_bus.as_deref() == Some(short.as_str()) && !text.is_empty() {
                short = text.clone();
            }
            let mut name_g = fit(&name, size, inner, colour).or_else(|| fit(&short, size.max(LABEL_MIN), inner, colour));
            // Still too long for its box: as much of it as fits, cut short.
            if name_g.is_none() && rect.height() >= 9.0 && inner >= 24.0 {
                let fs = LABEL_MIN.min(rect.height() - 2.0);
                let whole = painter.layout_no_wrap(short.clone(), FontId::monospace(fs), colour);
                let per = whole.size().x / short.chars().count().max(1) as f32;
                let n = ((inner / per).floor() as usize).saturating_sub(1);
                if n >= 3 {
                    name_g = Some(painter.layout_no_wrap(short.chars().take(n).collect::<String>() + "…", FontId::monospace(fs), colour));
                }
            }
            if name_g.is_none() {
                let rank = match it.role {
                    Role::Hub if d.major.contains(&it.part) => 4,
                    Role::Source => 3,
                    Role::Hub => 2,
                    _ => 1,
                };
                pending.push((rank * 100_000 + rect.width() as u32, p, name.clone(), style::text_of(st), rect));
            }
            match (name_g, sub_g) {
                (Some(g), Some(sg)) => {
                    let gap = 2.0 * z;
                    let total = g.size().y + gap + sg.size().y;
                    let top = p.y - total / 2.0;
                    painter.galley(Pos2::new(p.x - g.size().x / 2.0, top), g.clone(), style::text_of(st));
                    painter.galley(Pos2::new(p.x - sg.size().x / 2.0, top + g.size().y + gap), sg, faint);
                }
                (Some(g), None) => painter.galley(p - g.size() / 2.0, g, style::text_of(st)),
                _ => {}
            }
            if resp.hover_pos().is_some_and(|h| rect.expand(4.0).contains(h)) {
                hit = Some(it.part);
            }
        }
        pending.sort_by(|a, b| b.0.cmp(&a.0));
        let mut placed: Vec<Rect> = Vec::new();
        for (_, at_p, name, colour, own) in pending {
            let g = painter.layout_no_wrap(name, FontId::monospace(LABEL_MIN), colour);
            let r = Rect::from_center_size(at_p, g.size() + Vec2::new(6.0, 2.0));
            let own = own.expand(2.0);
            if placed.iter().any(|q| q.intersects(r)) || occupied.iter().any(|o| *o != own && o.intersects(r)) || !resp.rect.contains_rect(r) {
                continue;
            }
            painter.rect_filled(r, 3.0, style::PANEL);
            painter.galley(r.center() - g.size() / 2.0, g, colour);
            placed.push(r);
        }
        for (c, rad, vertical, t, st) in edge_labels {
            let colour = if st == State::Live || st == State::Off { faint } else { style::colour_of(st) };
            let gap = rad + 4.0 * z;
            let tries: [(Vec2, Align2); 4] = if vertical {
                [(Vec2::new(gap, 0.0), Align2::LEFT_CENTER), (Vec2::new(-gap, 0.0), Align2::RIGHT_CENTER), (Vec2::new(gap, -gap), Align2::LEFT_BOTTOM), (Vec2::new(gap, gap), Align2::LEFT_TOP)]
            } else {
                [(Vec2::new(0.0, -gap), Align2::CENTER_BOTTOM), (Vec2::new(0.0, gap), Align2::CENTER_TOP), (Vec2::new(gap, -gap), Align2::LEFT_BOTTOM), (Vec2::new(-gap, -gap), Align2::RIGHT_BOTTOM)]
            };
            'size: for k in [1.0f32, 0.82] {
                let Some(g) = fit(&t, 10.0 * z * k, f32::MAX, colour) else { break };
                for (off, al) in tries {
                    let r = al.anchor_size(c + off, g.size()).expand(1.5);
                    if !occupied.iter().any(|o| o.intersects(r)) {
                        painter.rect_filled(r, 2.0, style::PANEL);
                        painter.galley(r.min + Vec2::splat(1.5), g, colour);
                        occupied.push(r);
                        break 'size;
                    }
                }
            }
        }
        let resp = match hit {
            Some(i) => {
                let p = &a.checked.parts[i];
                let (_, _, text) = self.part_state(p);
                resp.on_hover_text_at_pointer(format!("{}  {}.{}\n{text}", p.id, p.domain, p.kind))
            }
            None => resp,
        };
        let _ = ink;
        let shift = ui.input(|i| i.modifiers.shift);
        if resp.drag_started() {
            if shift {
                self.connect_from = hit.filter(|&i| diagram::role(&a.checked.parts[i]).is_some_and(|r| r.0 == Role::Hub));
            } else {
                self.dragging = hit;
            }
        }
        if let (Some(from), Some(h)) = (self.connect_from, resp.hover_pos()) {
            if let Some(it) = d.items.iter().find(|x| x.part == from) {
                painter.add(egui::Shape::dashed_line(&[at(it.pos), h], Stroke::new(1.5f32, ink), 5.0, 4.0));
            }
        }
        if resp.drag_stopped() && self.connect_from.is_some() {
            let from = self.connect_from.take().unwrap();
            if let Some(to) = hit.filter(|&i| i != from && a.checked.parts[i].domain == a.checked.parts[from].domain && diagram::role(&a.checked.parts[i]).is_some_and(|r| r.0 == Role::Hub)) {
                let (fa, fb) = (a.checked.parts[from].id.clone(), a.checked.parts[to].id.clone());
                let kind = match a.checked.parts[from].domain {
                    "electrical" => "contactor",
                    "hydraulic" => "ptu",
                    _ => "valve",
                };
                let domain = a.checked.parts[from].domain;
                let file = self.model.as_ref().and_then(|m| m.sources.files.get(a.checked.parts[from].file)).map(|f| PathBuf::from(&f.0));
                if let Some(file) = file {
                    let id = format!("{fa}_{fb}");
                    self.history.touch(&file);
                    let r = edit::add(&file, domain, kind, &id).and_then(|_| edit::set(&file, &id, "between", &format!("[\"{fa}\", \"{fb}\"]")).map(|_| ()));
                    self.history.commit();
                    match r {
                        Ok(()) => {
                            self.status = format!("added {domain}.{kind} {id}");
                            self.reload();
                            self.selected = self.aircraft.as_ref().and_then(|a| a.checked.find(&id));
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
            return;
        }
        let mut fail_set = None;
        if let Some(i) = hit {
            let id = a.checked.parts[i].id.clone();
            let failed = self.get(&format!("{id}.failed")).is_some_and(|v| v != 0.0);
            let mut act = 0;
            resp.clone().context_menu(|ui| {
                if ui.button(if failed { "Repair" } else { "Fail" }).clicked() {
                    act = 1;
                    ui.close_menu();
                }
                if ui.button("Select").clicked() {
                    act = 2;
                    ui.close_menu();
                }
            });
            match act {
                1 => fail_set = Some((format!("{id}.failed"), if failed { 0.0 } else { 1.0 })),
                2 => self.selected = Some(i),
                _ => {}
            }
        }
        if resp.dragged() && self.connect_from.is_none() {
            match self.dragging {
                Some(i) => {
                    let id = a.checked.parts[i].id.clone();
                    let cur = d.items.iter().find(|x| x.part == i).map(|x| x.pos).unwrap_or_default();
                    let delta = resp.drag_delta();
                    self.pinned.insert(id, (cur.0 + delta.x / z, cur.1 + delta.y / z));
                }
                None => self.offset += resp.drag_delta(),
            }
        }
        if resp.drag_stopped() && self.dragging.take().is_some() {
            write_layout(&self.dir, &self.pinned);
        }
        if resp.clicked() && hit.is_some() {
            self.selected = hit;
            self.edits.clear();
        }
        painter.text(resp.rect.right_bottom() + Vec2::new(-8.0, -8.0), Align2::RIGHT_BOTTOM, "drag to move, shift-drag between two to connect, right-click to fail, scroll to zoom", FontId::proportional(10.5), faint);
        let rest = diagram::listed(&a.checked, domain);
        if !rest.is_empty() {
            let names: Vec<String> = rest.iter().map(|&i| a.checked.parts[i].id.clone()).collect();
            painter.text(resp.rect.left_bottom() + Vec2::new(8.0, -8.0), Align2::LEFT_BOTTOM, format!("Not drawn: {}", names.join(", ")), small, faint);
        }
        if let Some((n, v)) = fail_set {
            let cause = format!("{} {}", n.trim_end_matches(".failed"), if v != 0.0 { "failed" } else { "repaired" });
            self.inject(&n, v, cause);
        }
    }

    fn left(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.add_enabled(self.history.can_undo(), egui::Button::new("Undo")).clicked() {
                if let Err(e) = self.history.undo() {
                    self.status = e;
                }
                self.reload();
            }
            ui.toggle_value(&mut self.show_changes, "Changes");
        });
        if self.show_changes {
            let ch = self.history.changes();
            if ch.is_empty() {
                ui.small("Nothing changed since opening.");
            }
            for (f, add, del) in ch {
                ui.small(format!("{f}  +{add} -{del}"));
            }
        }
        egui::CollapsingHeader::new("Export to code").show(ui, |ui| {
            ui.add(egui::TextEdit::singleline(&mut self.export_dir).hint_text("new folder"));
            if ui.button("Export Rust crate").clicked() && !self.export_dir.trim().is_empty() {
                self.status = match easyacft_export::export(Path::new(&self.dir), Path::new(self.export_dir.trim()), None) {
                    Ok(s) => format!("wrote {}: {} parts, {} expressions, {} tests", s.out.display(), s.parts, s.expressions, s.tests),
                    Err(e) => e.lines().next().unwrap_or("export failed").to_owned(),
                };
            }
        });
        egui::CollapsingHeader::new("New aircraft").show(ui, |ui| {
            let t = templates();
            let name = |p: &PathBuf| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            egui::ComboBox::from_id_salt("template").selected_text(t.get(self.template).map(name).unwrap_or_else(|| "no templates found".into())).show_ui(ui, |ui| {
                for (i, p) in t.iter().enumerate() {
                    ui.selectable_value(&mut self.template, i, name(p));
                }
            });
            ui.add(egui::TextEdit::singleline(&mut self.new_dir).hint_text("new folder"));
            if ui.button("Create").clicked() && !self.new_dir.trim().is_empty() {
                if let Some(src) = t.get(self.template) {
                    match edit::copy_dir(src, Path::new(self.new_dir.trim())) {
                        Ok(()) => {
                            self.dir = self.new_dir.trim().to_owned();
                            self.aircraft = None;
                            self.history = edit::History::default();
                            self.reload();
                            self.fit = true;
                        }
                        Err(e) => self.status = e,
                    }
                }
            }
        });
        ui.separator();
        let search = ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search (Ctrl+K)"));
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::K)) {
            search.request_focus();
        }
        ui.separator();
        ui.label("Add part");
        egui::ComboBox::from_label("kind").selected_text(self.add_kind.clone()).show_ui(ui, |ui| {
            for s in catalog::list() {
                let k = format!("{}.{}", s.domain, s.kind);
                ui.selectable_value(&mut self.add_kind, k.clone(), k);
            }
        });
        ui.add(egui::TextEdit::singleline(&mut self.add_id).hint_text("id"));
        let files: Vec<String> = self.model.as_ref().map(|m| m.sources.files.iter().map(|f| f.0.clone()).collect()).unwrap_or_default();
        egui::ComboBox::from_label("file").selected_text(files.get(self.add_file).cloned().unwrap_or_default()).show_ui(ui, |ui| {
            for (i, f) in files.iter().enumerate() {
                ui.selectable_value(&mut self.add_file, i, f);
            }
        });
        if ui.button("Add").clicked() && !self.add_id.trim().is_empty() {
            if let (Some(f), Some((d, k))) = (files.get(self.add_file), self.add_kind.split_once('.')) {
                self.history.touch(Path::new(f));
                let r = edit::add(Path::new(f), d, k, self.add_id.trim());
                self.history.commit();
                match r {
                    Ok(()) => {
                        let id = self.add_id.trim().to_owned();
                        self.reload();
                        self.selected = self.aircraft.as_ref().and_then(|a| a.checked.find(&id));
                        self.status = format!("added {id}; fill in its settings and save");
                    }
                    Err(e) => self.status = e,
                }
            }
        }
        ui.separator();
        let Some(a) = &self.aircraft else { return };
        let q = self.search.to_lowercase();
        let mut pick = None;
        let domains: std::collections::BTreeSet<&str> = DOMAINS.iter().chain(MORE.iter()).copied().collect();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for d in domains {
                let items: Vec<(usize, &str)> = a.checked.parts.iter().enumerate().filter(|(_, p)| p.domain == d && (q.is_empty() || p.id.to_lowercase().contains(&q) || p.kind.contains(&q))).map(|(i, p)| (i, p.id.as_str())).collect();
                if items.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(d).default_open(!q.is_empty()).show(ui, |ui| {
                    for (i, id) in items {
                        if ui.selectable_label(self.selected == Some(i), id).clicked() {
                            pick = Some(i);
                        }
                    }
                });
            }
        });
        if pick.is_some() {
            self.selected = pick;
            self.edits.clear();
        }
    }

    fn engine_tab(&mut self, ui: &mut egui::Ui) {
        let Some(a) = &self.aircraft else { return };
        let base = PathBuf::from(&self.dir);
        let engines: Vec<easyacft_lang::CheckedPart> = a.checked.parts.iter().filter(|p| p.domain == "engine" || p.domain == "fadec" || p.domain == "apu").cloned().collect();
        if engines.is_empty() {
            ui.label("No engines in this aircraft.");
        }
        let ids: Vec<String> = engines.iter().map(|p| p.id.clone()).collect();
        if self.engine_view.as_ref().is_none_or(|v| !ids.contains(v)) {
            let sel = self.selected.map(|i| a.checked.parts[i].id.clone()).filter(|s| ids.contains(s));
            self.engine_view = sel.or_else(|| engines.iter().find(|p| p.kind == "gas_turbine").map(|p| p.id.clone())).or_else(|| ids.first().cloned());
        }
        ui.horizontal_wrapped(|ui| {
            for id in &ids {
                if ui.selectable_label(self.engine_view.as_ref() == Some(id), id).clicked() {
                    self.engine_view = Some(id.clone());
                    self.engine_pick = None;
                }
            }
        });
        let view = self.engine_view.clone();
        let mut actions: Vec<(String, f64, String)> = Vec::new();
        for p in engines.into_iter().filter(|p| Some(&p.id) == view.as_ref()) {
            ui.label(format!("{}.{}", p.domain, p.kind));
            if p.kind == "gas_turbine" {
                let params = easyacft_parts::engine::Params::from_part(&p, &base);
                ui.horizontal(|ui| {
                    ui.label("Show");
                    for (k, n) in [(0u8, "Auto"), (1, "Live sim"), (2, "Idle"), (3, "Take-off")] {
                        ui.selectable_value(&mut self.engine_show, k, n);
                    }
                });
                let w = ui.available_width().min(1600.0);
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, w * 0.88), Sense::click());
                let painter = ui.painter_at(rect);
                let mut live = self.engine_live(&p, ui.input(|i| i.time));
                live.selected = self.engine_pick.clone();
                let stopped = live.airflow < 0.01 * params.airflow;
                let mode = if self.engine_show == 0 { if stopped { 3 } else { 1 } } else { self.engine_show };
                if mode >= 2 {
                    let share = if mode == 2 { 0.06 } else { 1.0 };
                    let key = format!("{}|{mode}|{:?}", p.id, live.health);
                    let (g, n) = *self.steady.entry(key).or_insert_with(|| easyacft_parts::engine::steady(&params, live.health, share));
                    let pct = |v: f64| v * 100.0;
                    live.n = match params.spools {
                        1 => [pct(n[2]), 0.0, pct(n[2])],
                        2 => [pct(n[0]), 0.0, pct(n[2])],
                        _ => [pct(n[0]), pct(n[1]), pct(n[2])],
                    };
                    (live.t3, live.p3, live.tet, live.egt, live.fuel, live.thrust, live.airflow, live.shaft_power) = (g.t3, g.p3, g.t4, g.egt, g.wf, g.thrust, g.w, g.shaft);
                    live.running = g.wf > 0.0;
                    live.t0 = 288.15;
                    let what = if mode == 2 { "IDLE" } else { "TAKE-OFF" };
                    live.preview = Some(format!("PREVIEW: {what} STEADY STATE, SEA LEVEL STATIC{}", if stopped { "  (ENGINE NOT RUNNING IN THE SIM)" } else { "" }));
                }
                let hits = cutaway::draw(&painter, rect, &style::label(&p.id), &params, &live);
                let at = |pos: Option<Pos2>| pos.and_then(|q| hits.iter().filter(|h| h.rect.contains(q)).min_by(|a, b| a.rect.area().total_cmp(&b.rect.area())).cloned());
                let hovered = at(resp.hover_pos());
                let mut resp = resp;
                if let Some(h) = &hovered {
                    painter.rect_stroke(h.rect.expand(1.5), 4.0, Stroke::new(1.2f32, Color32::from_gray(230)));
                    let now = if h.graded { format!("damage {:.0} %", h.level * 100.0) } else if h.level != 0.0 { "failed".into() } else { "normal".into() };
                    resp = resp.on_hover_text_at_pointer(format!("{}\n{}\nclick: what it feeds   right-click: fail", h.label, now));
                }
                if resp.clicked() || resp.secondary_clicked() {
                    self.engine_pick = hovered.as_ref().map(|h| h.path.clone());
                }
                let menu_hit = self.engine_pick.as_ref().and_then(|pk| hits.iter().find(|h| &h.path == pk)).cloned();
                resp.context_menu(|ui| {
                    let Some(h) = &menu_hit else {
                        ui.label("Right-click a component");
                        return;
                    };
                    ui.strong(&h.label);
                    let options: &[(&str, f64)] = if h.graded { &[("Light damage, 25 %", 0.25), ("Heavy damage, 60 %", 0.6), ("Destroyed", 1.0), ("Repair", 0.0)] } else { &[("Fail", 1.0), ("Repair", 0.0)] };
                    for (name, v) in options {
                        if ui.button(*name).clicked() {
                            actions.push((h.path.clone(), *v, format!("{}: {}", h.label, name.to_lowercase())));
                            ui.close_menu();
                        }
                    }
                });
                ui.add_space(6.0);
                ui.columns(2, |cols| {
                    cols[0].strong("Failures");
                    let mut seen = std::collections::HashSet::new();
                    egui::Grid::new(format!("fail_{}", p.id)).striped(true).show(&mut cols[0], |ui| {
                        for h in hits.iter().filter(|h| seen.insert(h.path.clone())) {
                            let col = if h.level != 0.0 { if h.level >= 0.5 || !h.graded { style::RED } else { style::ORANGE } } else { style::GREY_TEXT };
                            if ui.selectable_label(self.engine_pick.as_ref() == Some(&h.path), egui::RichText::new(&h.label).color(col)).clicked() {
                                self.engine_pick = Some(h.path.clone());
                            }
                            ui.horizontal(|ui| {
                                let steps: &[(&str, f64)] = if h.graded { &[("0", 0.0), ("25", 0.25), ("60", 0.6), ("100", 1.0)] } else { &[("ok", 0.0), ("fail", 1.0)] };
                                for (name, v) in steps {
                                    if ui.selectable_label((h.level - v).abs() < 0.01, *name).clicked() && (h.level - v).abs() >= 0.01 {
                                        actions.push((h.path.clone(), *v, format!("{}: {}", h.label, if *v == 0.0 { "repaired".to_owned() } else if h.graded { format!("{name} % damage") } else { "failed".to_owned() })));
                                    }
                                }
                            });
                            ui.end_row();
                        }
                    });
                    if cols[0].button("Repair all").clicked() {
                        for h in hits.iter().filter(|h| h.level != 0.0) {
                            actions.push((h.path.clone(), 0.0, format!("{} repaired", p.id)));
                        }
                    }
                    self.downstream_ui(&mut cols[1], &p.id);
                });
                let g = easyacft_parts::engine::GasPath::new(params);
                egui::CollapsingHeader::new(format!("{} design point", p.id)).show(ui, |ui| {
                    egui::Grid::new(format!("d_{}", p.id)).show(ui, |ui| {
                        for (k, v) in engine_view::design_rows(&g) {
                            ui.label(k);
                            ui.label(v);
                            ui.end_row();
                        }
                    });
                });
            }
            let Some(spec) = catalog::find(p.domain, p.kind) else { continue };
            egui::CollapsingHeader::new(format!("{} values", p.id)).default_open(p.kind != "gas_turbine").show(ui, |ui| {
                egui::Grid::new(format!("eng_{}", p.id)).show(ui, |ui| {
                    for (n, t) in spec.outputs {
                        let v = self.get(&format!("{}.{n}", p.id)).unwrap_or(0.0);
                        ui.label(*n);
                        ui.label(unit(*t, v));
                        ui.end_row();
                    }
                });
            });
        }
        let many = actions.len() > 1;
        for (path, v, cause) in actions {
            if many {
                self.set(&path, v);
            } else {
                self.inject(&path, v, cause);
            }
        }
    }

    /// Green on, grey off, red failed, from a part's own outputs.
    fn part_light(&self, p: &easyacft_lang::CheckedPart) -> State {
        let (live, failed, text) = self.part_state(p);
        if failed {
            return State::Fail;
        }
        if text.contains("shed") || text.contains("tripped") {
            return State::Warn;
        }
        if live {
            return State::Live;
        }
        let on = |n: &str| self.get(&format!("{}.{n}", p.id)).is_some_and(|v| v != 0.0);
        if ["on", "active", "running", "powered", "available", "lit", "open", "closed", "ready", "valid", "supplied", "aligned"].iter().any(|n| on(n)) {
            State::Live
        } else {
            State::Off
        }
    }

    /// What reads the picked component's part, level by level, and every on/off value the last failure changed.
    fn downstream_ui(&mut self, ui: &mut egui::Ui, engine: &str) {
        self.readers();
        let Some(a) = &self.aircraft else { return };
        let Some(r) = &self.readers else { return };
        let root_id = self.engine_pick.as_ref().and_then(|p| p.split_once('.')).map(|(id, _)| id.to_owned()).unwrap_or_else(|| engine.to_owned());
        let Some(root) = a.checked.find(&root_id) else { return };
        let mut seen = vec![false; a.checked.parts.len()];
        seen[root] = true;
        let mut frontier = vec![root];
        let mut levels: Vec<Vec<usize>> = Vec::new();
        let mut count = 0;
        while levels.len() < 6 && count < 400 {
            let mut next: Vec<usize> = frontier.iter().flat_map(|&i| r[i].iter().copied()).filter(|&j| !std::mem::replace(&mut seen[j], true)).collect();
            if next.is_empty() {
                break;
            }
            next.sort_by_key(|&j| (a.checked.parts[j].domain, a.checked.parts[j].id.clone()));
            count += next.len();
            levels.push(next.clone());
            frontier = next;
        }
        let changed: std::collections::HashSet<usize> = self.cascade.as_ref().map(|c| c.first.iter().filter_map(|(s, _, _)| a.checked.slots.names.get(*s)).filter_map(|(n, _)| n.split_once('.')).filter_map(|(id, _)| a.checked.find(id)).collect()).unwrap_or_default();
        let mut pick = None;
        let mut clear = false;
        ui.strong(format!("Downstream of {}  ({} parts)", root_id, count));
        egui::ScrollArea::vertical().id_salt("down").max_height(300.0).show(ui, |ui| {
            for (d, lv) in levels.iter().enumerate() {
                ui.small(if d == 0 { "reads it directly".to_owned() } else { format!("{} steps away", d + 1) });
                ui.horizontal_wrapped(|ui| {
                    for &j in lv {
                        let p = &a.checked.parts[j];
                        let st = self.part_light(p);
                        let mut t = egui::RichText::new(&p.id).monospace().size(11.5).color(style::text_of(st));
                        if changed.contains(&j) {
                            t = t.underline().strong();
                        }
                        if ui.add(egui::Label::new(t).sense(Sense::click())).on_hover_text(format!("{}.{}", p.domain, p.kind)).clicked() {
                            pick = Some(j);
                        }
                    }
                });
            }
        });
        ui.add_space(6.0);
        if let Some(c) = &self.cascade {
            ui.strong(format!("Changed since: {}  ({})", c.cause, c.first.len()));
            egui::ScrollArea::vertical().id_salt("cascade").max_height(300.0).show(ui, |ui| {
                egui::Grid::new("cascade_grid").striped(true).show(ui, |ui| {
                    for (s, t, was) in c.first.iter().take(400) {
                        let Some((name, _)) = a.checked.slots.names.get(*s) else { continue };
                        let (was, now) = (*was, self.value(*s));
                        let bad = ["failed", "fault", "low", "lo_", "shed", "tripped", "warning", "caution", "fire", "alert", "lit", "overheat", "disagree"].iter().any(|k| name.contains(k));
                        let col = if now != 0.0 && bad { style::RED } else if now != 0.0 { style::GREEN } else { style::ORANGE };
                        ui.monospace(format!("+{t:.2} s"));
                        if ui.add(egui::Label::new(egui::RichText::new(name).monospace().color(col)).sense(Sense::click())).clicked() {
                            pick = name.split_once('.').and_then(|(id, _)| a.checked.find(id));
                        }
                        ui.monospace(format!("{} -> {}", if was != 0.0 { "on" } else { "off" }, if now != 0.0 { "on" } else { "off" }));
                        ui.end_row();
                    }
                });
            });
            clear = ui.small_button("Clear").clicked();
        } else {
            ui.small("Fail a component to record everything it changes.");
        }
        if clear {
            self.cascade = None;
        }
        if pick.is_some() {
            self.selected = pick;
            self.edits.clear();
        }
    }

    fn engine_live(&self, e: &easyacft_lang::CheckedPart, time: f64) -> cutaway::Live {
        let Some(a) = &self.aircraft else { return Default::default() };
        let g = |n: &str| self.get(&format!("{}.{n}", e.id)).unwrap_or(0.0);
        let on = |id: &str, n: &str| self.get(&format!("{id}.{n}")).is_some_and(|v| v != 0.0);
        let spools = e.num("spools").clamp(1.0, 3.0) as usize;
        let n = match spools {
            1 => [g("n1"), 0.0, g("n1")],
            2 => [g("n1"), 0.0, g("n2")],
            _ => [g("n1"), g("n2"), g("n3")],
        };
        let t0 = e.part("air").and_then(|i| self.get(&format!("{}.temperature", a.checked.parts[i].id))).filter(|t| *t > 150.0).unwrap_or(288.15);
        let me = a.checked.find(&e.id);
        let fadec = a.checked.parts.iter().find(|p| p.domain == "fadec" && p.part("engine") == me);
        let mentions = |p: &easyacft_lang::CheckedPart| p.exprs.iter().any(|(_, src)| src.match_indices(&format!("{}.", e.id)).any(|(i, _)| i == 0 || !src.as_bytes()[i - 1].is_ascii_alphanumeric() && src.as_bytes()[i - 1] != b'_'));
        let st = |p: &easyacft_lang::CheckedPart, ok: &str| if on(&p.id, "failed") { State::Fail } else if on(&p.id, ok) { State::Live } else { State::Off };
        let digits: String = e.id.chars().filter(|c| c.is_ascii_digit()).collect();
        let health = easyacft_parts::engine::Health {
            comp: [g("fan_damage"), g("lpc_damage"), g("ipc_damage"), g("hpc_damage")],
            burner: g("combustor_damage"),
            turb: [g("hpt_damage"), g("ipt_damage"), g("lpt_damage")],
            surge: g("surge") != 0.0,
            leak: g("case_leak"),
            seized: [g("seized_lp") != 0.0, g("seized_ip") != 0.0, g("seized_hp") != 0.0],
        };
        let mut l = cutaway::Live { id: e.id.clone(), n, running: g("running") != 0.0, failed: g("failed") != 0.0, health, t0, t3: g("t3"), p3: g("p3"), tet: g("tet"), egt: g("egt"), fuel: g("fuel_flow"), thrust: g("thrust"), airflow: g("airflow"), shaft_power: g("shaft_power"), time, ..Default::default() };
        if let Some(f) = fadec {
            l.eec_id = Some(f.id.clone());
            l.starter = on(&f.id, "starter");
            l.ignition = on(&f.id, "ignition");
            l.reverser = self.get(&format!("{}.reverser", f.id)).unwrap_or(0.0);
            l.eec = if on(&f.id, "failed") { State::Fail } else if on(&f.id, "fault") { State::Warn } else if self.get(&format!("{}.state", f.id)).unwrap_or(0.0) > 0.0 || l.running { State::Live } else { State::Off };
        }
        for p in &a.checked.parts {
            match (p.domain, p.kind) {
                ("electrical", "generator") if mentions(p) => l.gens.push((p.id.clone(), st(p, "ready"))),
                ("hydraulic", "pump") if mentions(p) => l.pumps.push((p.id.clone(), st(p, "running"))),
                ("pneumatic", "source") if mentions(p) => l.bleeds.push((p.id.clone(), st(p, "available"))),
                ("pneumatic", "consumer") if !digits.is_empty() && p.id.contains("nacelle") && p.id.contains(&digits) => l.anti_ice = Some((p.id.clone(), st(p, "supplied"))),
                ("fire", "zone") if p.id.starts_with(&e.id) => {
                    if on(&p.id, "burning") || on(&p.id, "detected") {
                        l.fire = State::Fail;
                    }
                    let sw = p.exprs.iter().filter(|(k, _)| *k == "fire_when").flat_map(|(_, src)| src.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.')).map(str::to_owned).collect::<Vec<_>>()).filter_map(|t| t.split_once('.').map(|(id, _)| id.to_owned())).find(|id| a.checked.find(id).is_some_and(|i| a.checked.parts[i].domain == "cockpit"));
                    if sw.is_some() {
                        l.fire_switch = sw;
                    }
                }
                _ => {}
            }
        }
        l.bleeds.reverse();
        l
    }

    fn computers_tab(&mut self, ui: &mut egui::Ui) {
        let Some(a) = &self.aircraft else { return };
        let rows: Vec<(usize, String, &'static str, &'static str)> = a.checked.parts.iter().enumerate().filter(|(_, p)| matches!(p.domain, "computer" | "data" | "sensor")).map(|(i, p)| (i, p.id.clone(), p.domain, p.kind)).collect();
        if rows.is_empty() {
            ui.label("No computers, data links or sensors in this aircraft.");
            return;
        }
        let mut pick = None;
        egui::Grid::new("computers").striped(true).show(ui, |ui| {
            ui.strong("part");
            ui.strong("kind");
            ui.strong("state");
            ui.end_row();
            for (i, id, d, k) in rows {
                if ui.link(&id).clicked() {
                    pick = Some(i);
                }
                ui.label(format!("{d}.{k}"));
                let g = |n: &str| self.get(&format!("{id}.{n}"));
                let on = |n: &str| g(n).is_some_and(|v| v != 0.0);
                let state = if on("failed") {
                    "failed".to_owned()
                } else {
                    match (d, k) {
                        ("computer", "unit") if on("booting") => "booting".into(),
                        ("computer", "unit") if on("running") => format!("running, channel {:.0}{}", g("channel").unwrap_or(0.0), if on("bad_data") { ", bad data" } else if on("frozen") { ", frozen" } else { "" }),
                        ("computer", "unit") => if on("powered") { "powered".into() } else { "off".into() },
                        ("data", "link") => format!("{} {:.3}, status {:.0}, age {:.2} s", if on("valid") { "valid" } else { "stale" }, g("value").unwrap_or(0.0), g("status").unwrap_or(0.0), g("age").unwrap_or(0.0)),
                        ("computer", "air_data") => format!("{} {:.0} kt {:.0} ft", if on("valid") { "valid" } else { "invalid" }, g("ias").unwrap_or(0.0) * 1.943_844, g("altitude").unwrap_or(0.0) / 0.3048),
                        ("computer", "inertial") => if on("aligning") { "aligning".into() } else if on("aligned") { format!("aligned, drift {:.0} m", g("position_error").unwrap_or(0.0)) } else { "off".into() },
                        _ => String::new(),
                    }
                };
                ui.label(state);
                ui.end_row();
            }
        });
        if pick.is_some() {
            self.selected = pick;
            self.edits.clear();
        }
    }

    fn health(&self, ui: &mut egui::Ui) {
        let (Some(a), Some(m)) = (&self.aircraft, &self.model) else { return };
        let all_exprs: String = a.checked.parts.iter().flat_map(|p| p.exprs.iter().map(|e| e.1.clone())).collect::<Vec<_>>().join(" ");
        let bound: Vec<&str> = m.bindings.iter().map(|b| b.path.as_str()).collect();
        let mut issues = Vec::new();
        for p in a.checked.parts.iter().filter(|p| p.domain == "cockpit") {
            let used = all_exprs.contains(&format!("{}.", p.id)) || bound.iter().any(|b| b.starts_with(&format!("{}.", p.id)));
            if !used {
                issues.push(format!("{} is not used by anything and has no binding", p.id));
            }
        }
        for p in a.checked.parts.iter().filter(|p| p.domain == "electrical" && p.kind == "bus") {
            if self.get(&format!("{}.powered", p.id)) == Some(0.0) && a.time > 5.0 {
                issues.push(format!("{} is not powered", p.id));
            }
        }
        for p in a.checked.parts.iter().filter(|p| p.kind == "breaker") {
            if self.get(&format!("{}.tripped", p.id)) == Some(1.0) {
                issues.push(format!("{} has tripped: its load is above its rating", p.id));
            }
        }
        for p in a.checked.parts.iter().filter(|p| p.domain == "fuel" && p.kind == "feed") {
            if self.get(&format!("{}.starved", p.id)) == Some(1.0) {
                issues.push(format!("{} is starved of fuel", p.id));
            }
        }
        if issues.is_empty() {
            ui.label("Nothing to report.");
        }
        for i in issues {
            ui.label(i);
        }
    }

    fn tables_tab(&mut self, ui: &mut egui::Ui) {
        if self.tables.is_empty() {
            ui.label("No .csv tables in this aircraft folder.");
        }
        let mut save = None;
        for (k, (path, rows)) in self.tables.iter_mut().enumerate() {
            ui.collapsing(path.display().to_string(), |ui| {
                egui::Grid::new(format!("t{k}")).show(ui, |ui| {
                    for row in rows.iter_mut() {
                        for cell in row.iter_mut() {
                            ui.add(egui::TextEdit::singleline(cell).desired_width(60.0));
                        }
                        ui.end_row();
                    }
                });
                if ui.button("Save table").clicked() {
                    save = Some(k);
                }
                let nums: Vec<Vec<f64>> = rows.iter().filter_map(|r| r.iter().map(|c| c.parse::<f64>().ok()).collect::<Option<Vec<f64>>>()).collect();
                if nums.len() >= 2 && nums[0].len() >= 2 {
                    egui_plot::Plot::new(format!("tp{k}")).height(180.0).show(ui, |pu| {
                        for col in 1..nums[0].len() {
                            let pts: Vec<[f64; 2]> = nums.iter().filter(|r| r.len() > col).map(|r| [r[0], r[col]]).collect();
                            pu.line(egui_plot::Line::new(egui_plot::PlotPoints::from(pts)).color(Color32::from_gray(40)));
                        }
                    });
                }
            });
        }
        if let Some(k) = save {
            let (path, rows) = &self.tables[k];
            let text: String = rows.iter().map(|r| r.join(", ") + "\n").collect();
            self.history.touch(path);
            let r = std::fs::write(path, text);
            self.history.commit();
            match r {
                Ok(()) => self.reload(),
                Err(e) => self.status = e.to_string(),
            }
        }
    }

    fn fit_tab(&mut self, ui: &mut egui::Ui) {
        ui.label("Test points: [[point]] tables with thrust (share), fuel_flow and egt");
        ui.add(egui::TextEdit::multiline(&mut self.fit_points).font(egui::TextStyle::Monospace).desired_rows(8));
        ui.horizontal(|ui| {
            ui.label("Tune");
            ui.text_edit_singleline(&mut self.fit_tune);
        });
        let engines: Vec<String> = self.aircraft.as_ref().map(|a| a.checked.parts.iter().filter(|p| p.kind == "gas_turbine").map(|p| p.id.clone()).collect()).unwrap_or_default();
        for e in engines {
            if ui.button(format!("Fit {e}")).clicked() {
                self.fit_out = run_fit(self.aircraft.as_ref().unwrap(), &e, &self.dir, &self.fit_points, &self.fit_tune);
            }
        }
        ui.monospace(&self.fit_out);
    }

    fn compare_tab(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Variant folder");
            ui.text_edit_singleline(&mut self.compare_dir);
            if ui.button("Compare").clicked() {
                self.compare_out = compare(self.model.as_ref(), &load(Path::new(&self.compare_dir)).0);
            }
        });
        egui::ScrollArea::vertical().show(ui, |ui| {
            for l in &self.compare_out {
                ui.monospace(l);
            }
        });
    }
}

fn scan_tables(dir: &Path) -> Vec<(PathBuf, Vec<Vec<String>>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "csv") {
                let rows = std::fs::read_to_string(&p).unwrap_or_default().lines().filter(|l| !l.trim().is_empty()).map(|l| l.split(',').map(|c| c.trim().to_owned()).collect()).collect();
                out.push((p, rows));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn run_fit(a: &Aircraft, engine: &str, dir: &str, points: &str, tune: &str) -> String {
    use easyacft_parts::engine::{fit, fit_error, FitPoint, Params};
    let Some(p) = a.checked.parts.iter().find(|p| p.id == engine) else { return "no such engine".into() };
    let params = Params::from_part(p, Path::new(dir));
    let doc = match points.parse::<toml_edit::DocumentMut>() {
        Ok(d) => d,
        Err(e) => return e.to_string(),
    };
    let q = |v: Option<&toml_edit::Item>, dim| v.and_then(|x| x.as_str()).and_then(|s| easyacft_core::parse_quantity(s).ok()).filter(|x| x.dim == dim).map(|x| x.si);
    let pts: Vec<FitPoint> = doc
        .get("point")
        .and_then(|p| p.as_array_of_tables())
        .map(|arr| arr.iter().map(|t| FitPoint { thrust_share: t.get("thrust").and_then(|x| x.as_float().or(x.as_integer().map(|i| i as f64))).unwrap_or(1.0), fuel_flow: q(t.get("fuel_flow"), easyacft_core::Dim::MASS_FLOW), egt: q(t.get("egt"), easyacft_core::Dim::TEMPERATURE) }).collect())
        .unwrap_or_default();
    if pts.is_empty() {
        return "no [[point]] tables".into();
    }
    let names: Vec<&str> = tune.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let tuned = fit(params.clone(), &pts, &names, 8);
    let mut out = String::new();
    for (title, prm) in [("as written", &params), ("tuned", &tuned)] {
        let (err, rows) = fit_error(prm, &pts);
        out += &format!("{title}: error {err:.5}\n");
        for (pt, (wf, egt, _)) in pts.iter().zip(rows) {
            out += &format!("  {:>4.0} %  fuel {wf:.3} kg/s  target {}  EGT {:.0} °C\n", pt.thrust_share * 100.0, pt.fuel_flow.map_or("-".into(), |t| format!("{t:.3}")), egt - 273.15);
        }
    }
    out += &format!("hpc_efficiency = {:.4}\nhpt_efficiency = {:.4}\nlpt_efficiency = {:.4}\nfan_efficiency = {:.4}\n", tuned.eta_c[3], tuned.eta_t[0], tuned.eta_t[2], tuned.eta_c[0]);
    out
}

fn compare(a: Option<&Model>, b: &Model) -> Vec<String> {
    let Some(a) = a else { return vec!["open an aircraft first".into()] };
    let mut out = Vec::new();
    for p in &a.parts {
        match b.part(&p.id) {
            None => out.push(format!("- {} only here", p.id)),
            Some(q) => {
                for f in &p.fields {
                    match q.get(&f.key) {
                        Some(g) if g.value == f.value => {}
                        Some(g) => out.push(format!("~ {}.{}: {:?} -> {:?}", p.id, f.key, f.value, g.value)),
                        None => out.push(format!("~ {}.{}: {:?} -> default", p.id, f.key, f.value)),
                    }
                }
            }
        }
    }
    for q in &b.parts {
        if a.part(&q.id).is_none() {
            out.push(format!("+ {} only in the variant", q.id));
        }
    }
    if out.is_empty() {
        out.push("no differences".into());
    }
    out
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.tick();
        if let Some((path, n)) = &mut self.shot {
            *n += 1;
            if *n == 8 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
            }
            let path = path.clone();
            let got = ctx.input(|i| i.raw.events.iter().find_map(|e| if let egui::Event::Screenshot { image, .. } = e { Some(image.clone()) } else { None }));
            if let Some(img) = got {
                let rgba: Vec<u8> = img.pixels.iter().flat_map(|c| c.to_array()).collect();
                let _ = std::fs::write(&path, png::encode(img.size[0] as u32, img.size[1] as u32, &rgba));
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F1)) {
            self.help = !self.help;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::F11)) || std::mem::take(&mut self.toggle_full) {
            self.full = !self.full;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.full));
            // Full screen draws at one screen pixel per unit, so a 4K monitor gives the diagram all of its pixels.
            if self.full {
                self.saved_ppp = Some(ctx.pixels_per_point());
                ctx.set_pixels_per_point(1.0);
            } else if let Some(p) = self.saved_ppp.take() {
                ctx.set_pixels_per_point(p);
            }
            self.fit = true;
            self.zoomed = false;
        }
        let bare = self.full && self.tab == Tab::Diagram;
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Aircraft folder");
                ui.add(egui::TextEdit::singleline(&mut self.dir).desired_width(320.0));
                if ui.button("Open").clicked() {
                    self.aircraft = None;
                    self.reload();
                }
                if ui.button("Reload").clicked() {
                    self.reload();
                }
                ui.separator();
                if ui.button(if self.running { "Pause" } else { "Run" }).clicked() {
                    self.running = !self.running;
                }
                if ui.button("Step").clicked() {
                    if let Some(a) = &mut self.aircraft {
                        a.step(0.05);
                    }
                }
                egui::ComboBox::from_label("speed").selected_text(format!("{}x", self.speed)).show_ui(ui, |ui| {
                    for s in [0.25, 1.0, 4.0, 10.0] {
                        ui.selectable_value(&mut self.speed, s, format!("{s}x"));
                    }
                });
                if let Some(a) = &self.aircraft {
                    ui.label(format!("t = {:.1} s", a.time));
                }
                ui.separator();
                match self.trial.is_some() {
                    false => {
                        if ui.button("Try").on_hover_text("snapshot now; keep or discard what you do next").clicked() {
                            self.trial = self.aircraft.as_ref().map(|a| a.snapshot());
                        }
                    }
                    true => {
                        if ui.button("Keep").clicked() {
                            self.trial = None;
                        }
                        if ui.button("Discard").clicked() {
                            if let (Some(a), Some(s)) = (&mut self.aircraft, self.trial.take()) {
                                a.restore(&s);
                            }
                        }
                    }
                }
                if self.recording.is_none() {
                    if ui.button("Record test").clicked() {
                        self.recording = Some(Vec::new());
                        self.record_start = self.aircraft.as_ref().map_or(0.0, |a| a.time);
                    }
                } else if ui.button("Stop and save test").clicked() {
                    let steps = self.recording.take().unwrap_or_default();
                    if let Some(a) = &self.aircraft {
                        let end = a.time - self.record_start;
                        let mut text = String::new();
                        for (t, n, v) in steps {
                            text += &format!("[[step]]\nat = \"{t:.2} s\"\nset = {{ \"{n}\" = {v} }}\n\n");
                        }
                        for (i, p) in a.checked.parts.iter().enumerate() {
                            if p.domain == "electrical" && p.kind == "bus" {
                                let on = a.slots[a.checked.slots.index[&format!("{}.powered", p.id)]] != 0.0;
                                text += &format!("[[expect]]\nat = \"{end:.2} s\"\nthat = \"{}{}.powered\"\n\n", if on { "" } else { "!" }, p.id);
                            }
                            let _ = i;
                        }
                        let dir = Path::new(&self.dir).join("tests");
                        let _ = std::fs::create_dir_all(&dir);
                        let n = std::fs::read_dir(&dir).map(|r| r.count()).unwrap_or(0) + 1;
                        let f = dir.join(format!("recorded-{n}.toml"));
                        self.status = match std::fs::write(&f, text) {
                            Ok(()) => format!("saved {}; run it with easyacft test", f.display()),
                            Err(e) => e.to_string(),
                        };
                    }
                }
                ui.separator();
                let attached = self.live.as_ref().is_some_and(|l| l.connected);
                if ui.button(if attached { "Detach from X-Plane" } else { "Attach to X-Plane" }).clicked() {
                    self.live = if attached { None } else { live::Client::connect() };
                    if self.live.is_none() && !attached {
                        self.status = "X-Plane is not running an EasyAcft aircraft".into();
                    }
                }
                ui.label(&self.status);
            });
            ui.horizontal(|ui| {
                for (t, n) in [(Tab::Diagram, "Systems"), (Tab::Engine, "Engines"), (Tab::Computers, "Computers"), (Tab::Plot, "Plots"), (Tab::Log, "Maintenance log"), (Tab::Health, "Health"), (Tab::Tables, "Tables"), (Tab::Fit, "Engine fit"), (Tab::Compare, "Compare")] {
                    ui.selectable_value(&mut self.tab, t, n);
                }
                if self.tab == Tab::Diagram {
                    ui.separator();
                    for (i, d) in DOMAINS.iter().enumerate().take(4) {
                        if ui.selectable_value(&mut self.domain, i, *d).clicked() {
                            self.fit = true;
                        }
                    }
                    if ui.selectable_label(false, "engines").clicked() {
                        self.tab = Tab::Engine;
                    }
                    if ui.button("Fit").clicked() {
                        self.fit = true;
                        self.zoomed = false;
                    }
                    if ui.selectable_label(self.full, "Full screen").on_hover_text("F11: the diagram alone, one screen pixel per unit").clicked() {
                        self.toggle_full = true;
                    }
                    let mut all = !self.fold;
                    if ui.checkbox(&mut all, "Show every breaker").on_hover_text("Off: each bus lists its breakers and users when you click it").changed() {
                        self.fold = !all;
                        self.fit = true;
                    }
                }
                if self.timeline.len() > 1 {
                    ui.separator();
                    let last = self.timeline.len() - 1;
                    let mut k = self.scrub.unwrap_or(last);
                    let label = format!("replay {:.0} s", self.timeline[k].0 - self.timeline[last].0);
                    if ui.add(egui::Slider::new(&mut k, 0..=last).show_value(false).text(label)).changed() {
                        self.scrub = Some(k);
                        if let Some(a) = &mut self.aircraft {
                            a.restore(&self.timeline[k].1);
                        }
                    }
                    if self.scrub.is_some() && ui.button("Go on from here").clicked() {
                        let k = self.scrub.take().unwrap();
                        self.timeline.truncate(k + 1);
                    }
                }
            });
        });
        if !self.report.is_empty() && !bare {
            egui::TopBottomPanel::bottom("report").resizable(true).show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    ui.monospace(&self.report);
                });
            });
        }
        if !bare {
            egui::SidePanel::left("left").default_width(220.0).show(ctx, |ui| self.left(ui));
            egui::SidePanel::right("right").default_width(360.0).max_width(460.0).show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui));
            });
        }
        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Diagram => self.draw_diagram(ui),
            Tab::Engine => {
                egui::ScrollArea::vertical().show(ui, |ui| self.engine_tab(ui));
            }
            Tab::Plot => {
                if ui.button("Clear plots").clicked() {
                    self.plots.clear();
                }
                egui_plot::Plot::new("plot").legend(egui_plot::Legend::default()).show(ui, |pu| {
                    for (n, pts) in &self.plots {
                        pu.line(egui_plot::Line::new(egui_plot::PlotPoints::from(pts.clone())).name(n).color(Color32::DARK_GRAY));
                    }
                });
            }
            Tab::Log => {
                if let Some(a) = &self.aircraft {
                    if a.log.is_empty() {
                        ui.label("No faults yet.");
                    }
                    for e in &a.log {
                        ui.monospace(format!("{:>8.1} s  {}  {}", e.time, e.what, if e.on_ground { "ground" } else { "air" }));
                    }
                }
            }
            Tab::Health => self.health(ui),
            Tab::Tables => {
                egui::ScrollArea::vertical().show(ui, |ui| self.tables_tab(ui));
            }
            Tab::Fit => self.fit_tab(ui),
            Tab::Computers => {
                egui::ScrollArea::vertical().show(ui, |ui| self.computers_tab(ui));
            }
            Tab::Compare => self.compare_tab(ui),
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
}

fn visuals() -> egui::Visuals {
    let mut v = egui::Visuals::dark();
    v.panel_fill = style::BG;
    v.window_fill = style::PANEL;
    v.extreme_bg_color = style::FILL;
    v.faint_bg_color = style::PANEL;
    v.selection.bg_fill = Color32::from_rgb(58, 61, 66);
    v.selection.stroke = Stroke::new(1.0f32, Color32::from_gray(230));
    v.hyperlink_color = Color32::from_gray(225);
    v.widgets.noninteractive.bg_fill = style::PANEL;
    v.widgets.inactive.bg_fill = Color32::from_rgb(45, 48, 53);
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(45, 48, 53);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(58, 61, 66);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(70, 74, 80);
    v
}

fn export(args: &[String]) -> bool {
    let [dir, domain, out, rest @ ..] = args else {
        eprintln!("usage: easyacft-studio --svg <aircraft folder> <domain> <out.svg> [seconds]");
        return false;
    };
    let mut a = match Aircraft::load(Path::new(dir)) {
        Ok((a, _)) => a,
        Err(r) => {
            eprintln!("{}", r.render());
            return false;
        }
    };
    let secs: f64 = rest.first().and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let mut t = 0.0;
    while t < secs {
        a.step(0.05);
        t += 0.05;
    }
    for s in rest.iter().skip(1) {
        if let Some((n, v)) = s.split_once('=') {
            a.set(n, v.parse().unwrap_or(0.0));
        }
    }
    if rest.len() > 1 {
        for _ in 0..20 {
            a.step(0.05);
        }
    }
    let svg = svg::render(&a.checked, domain, &read_layout(dir), |n| a.get(n));
    match std::fs::write(out, svg) {
        Ok(()) => true,
        Err(e) => {
            eprintln!("{e}");
            false
        }
    }
}

/// Smallest a name is drawn when its box is too small to hold it.
const LABEL_MIN: f32 = 8.0;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--svg") {
        std::process::exit(if export(&args[1..]) { 0 } else { 1 });
    }
    let dir = args.first().filter(|a| !a.starts_with("--")).cloned().unwrap_or_else(last_dir);
    let flag = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let shot = flag("--shot");
    let tab = flag("--tab");
    let domain = flag("--domain");
    let select = flag("--select");
    let scenario = flag("--scenario");
    let run: f64 = flag("--run").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let pairs = |f: &str| -> Vec<(String, f64)> { args.windows(2).filter(|w| w[0] == f).filter_map(|w| w[1].split_once('=').map(|(n, v)| (n.to_owned(), v.parse().unwrap_or(0.0)))).collect() };
    let sets = pairs("--set");
    let fails = pairs("--fail");
    let pick = flag("--pick");
    let zoom: Option<f32> = flag("--zoom").and_then(|s| s.parse().ok());
    let size: [f32; 2] = flag("--size").and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse().ok()?, h.parse().ok()?]))).unwrap_or([1400.0, 860.0]);
    let bare = args.iter().any(|a| a == "--bare");
    let mut viewport = egui::ViewportBuilder::default().with_inner_size(size).with_title("EasyAcft Studio");
    if shot.is_some() {
        viewport = viewport.with_position([-20000.0, -20000.0]).with_taskbar(false).with_decorations(false);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "EasyAcft Studio",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
            cc.egui_ctx.set_visuals_of(egui::Theme::Dark, visuals());
            let mut app = App::new(dir);
            if bare {
                app.full = true;
                cc.egui_ctx.set_pixels_per_point(1.0);
            }
            for (n, v) in &sets {
                app.set(n, *v);
            }
            if let (Some(a), Some(f)) = (&mut app.aircraft, &scenario) {
                if let Err(e) = std::fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|t| easyacft_parts::scenario::run(a, &t, 0.05)) {
                    app.status = e;
                }
            }
            for (n, v) in &fails {
                app.inject(n, *v, format!("{n} = {v}"));
            }
            let mut t = 0.0;
            while t < run {
                if let Some(a) = &mut app.aircraft {
                    a.step(0.05);
                }
                app.note_cascade();
                t += 0.05;
            }
            app.engine_pick = pick;
            if let Some(z) = zoom {
                app.zoom = z;
                app.zoomed = true;
                app.fit = false;
                app.fit_size = Vec2::new(1.0, 1.0);
            }
            if let Some(d) = domain.and_then(|d| DOMAINS.iter().position(|x| *x == d)) {
                app.domain = d;
            }
            app.tab = match tab.as_deref() {
                Some("engine") => Tab::Engine,
                Some("plot") => Tab::Plot,
                Some("log") => Tab::Log,
                Some("health") => Tab::Health,
                Some("tables") => Tab::Tables,
                Some("fit") => Tab::Fit,
                Some("compare") => Tab::Compare,
                Some("computers") => Tab::Computers,
                _ => Tab::Diagram,
            };
            app.selected = select.and_then(|id| app.aircraft.as_ref().and_then(|a| a.checked.find(&id)));
            app.shot = shot.map(|p| (p, 0));
            Ok(Box::new(app))
        }),
    )
}
