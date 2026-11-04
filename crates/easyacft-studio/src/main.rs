//! EasyAcft Studio: open an aircraft folder, watch every system run, edit its files.

mod diagram;
mod edit;
mod engine_view;
mod inspector;
mod live;
mod png;
mod svg;
mod why;

use diagram::{Diagram, Role, DOMAINS};
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
                self.aircraft = Some(fresh);
            }
            Err(report) => {
                self.report = report.render();
                self.status = format!("{} errors; the last good version keeps running", report.errors());
            }
        }
        self.tables = scan_tables(&path);
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
        let d: Diagram = diagram::build(&a.checked, domain, &self.pinned);
        let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        if !self.zoomed && self.fit_size != resp.rect.size() {
            self.fit = true;
        }
        if self.fit {
            self.fit_size = resp.rect.size();
            let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            let pts = d.items.iter().flat_map(|it| [(it.pos.0 - it.size.0 / 2.0, it.pos.1 - it.size.1 / 2.0 - 16.0), (it.pos.0 + it.size.0 / 2.0, it.pos.1 + it.size.1 / 2.0 + 16.0)]).chain(d.edges.iter().flat_map(|e| e.path.iter().copied()));
            for (px, py) in pts {
                (x0, y0, x1, y1) = (x0.min(px), y0.min(py), x1.max(px), y1.max(py));
            }
            if x0 <= x1 {
                let pad = 20.0;
                self.zoom = ((resp.rect.width() - 2.0 * pad) / (x1 - x0)).min((resp.rect.height() - 2.0 * pad) / (y1 - y0)).clamp(0.3, 1.5);
                self.offset = Vec2::new(pad - x0 * self.zoom, pad - y0 * self.zoom);
            }
            self.fit = false;
        }
        if let Some(h) = resp.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                let old = self.zoom;
                self.zoom = (self.zoom * (1.0 + scroll * 0.002)).clamp(0.2, 3.0);
                self.zoomed = true;
                let anchor = h - resp.rect.min - self.offset;
                self.offset -= anchor * (self.zoom / old - 1.0);
            }
        }
        let z = self.zoom;
        let origin = resp.rect.min.to_vec2() + self.offset;
        let (ink, faint, fill) = (self.theme.ink, self.theme.faint, self.theme.fill);
        painter.rect_filled(resp.rect, 0.0, self.theme.paper);
        let labels = z >= 0.55;
        let font = FontId::proportional(12.0 * z);
        let small = FontId::proportional(10.5 * z);
        let at = |p: (f32, f32)| Pos2::new(p.0 * z, p.1 * z) + origin;
        let stroke = |live: bool, failed: bool| Stroke::new(if live { 2.6f32 } else { 1.0 }, if failed || live { ink } else { faint });
        let line = |pts: &[Pos2], s: Stroke, failed: bool| {
            if failed {
                painter.extend(egui::Shape::dashed_line(pts, s, 6.0, 4.0));
            } else {
                painter.add(egui::Shape::line(pts.to_vec(), s));
            }
        };
        for (part, l) in &d.links {
            let (live, failed, _) = self.part_state(&a.checked.parts[*part]);
            line(&[at(l[0]), at(l[1])], stroke(live, failed), failed);
        }
        let mut hit = None;
        let mut edge_labels = Vec::new();
        for e in &d.edges {
            let (live, failed, text) = self.part_state(&a.checked.parts[e.part]);
            let pts: Vec<Pos2> = e.path.iter().map(|&p| at(p)).collect();
            line(&pts, stroke(live, failed), failed);
            let c = at(e.mark);
            let r = Rect::from_center_size(c, Vec2::splat(14.0 * z));
            painter.rect_filled(r, 2.0, fill);
            let sel = self.selected == Some(e.part);
            painter.rect_stroke(r, 2.0, if sel { Stroke::new(3.0f32, ink) } else { stroke(live, failed) });
            if live {
                painter.line_segment([c - Vec2::new(5.0 * z, 0.0), c + Vec2::new(5.0 * z, 0.0)], stroke(true, false));
            }
            let (tp, al) = if e.vertical { (c + Vec2::new(11.0 * z, 0.0), Align2::LEFT_CENTER) } else { (c - Vec2::new(0.0, 9.0 * z), Align2::CENTER_BOTTOM) };
            if labels {
                edge_labels.push((tp, al, format!("{} {text}", a.checked.parts[e.part].id)));
            }
            if resp.hover_pos().is_some_and(|h| r.expand(5.0).contains(h)) {
                hit = Some(e.part);
            }
        }
        for (tp, al, t) in edge_labels {
            let g = painter.layout_no_wrap(t, small.clone(), faint);
            let r = al.anchor_size(tp, g.size()).expand(1.5);
            painter.rect_filled(r, 2.0, self.theme.paper);
            painter.galley(r.min + Vec2::splat(1.5), g, faint);
        }
        for it in &d.items {
            let p = at(it.pos);
            let part = &a.checked.parts[it.part];
            let (live, failed, text) = self.part_state(part);
            let sel = self.selected == Some(it.part);
            let s = Stroke::new(if sel { 3.0 } else { stroke(live, failed).width }, if live || sel || failed { ink } else { faint });
            let rect = Rect::from_center_size(p, Vec2::new(it.size.0, it.size.1) * z);
            let round = if it.role == Role::Source { it.size.1 * z / 2.0 } else { 3.0 };
            painter.rect_filled(rect, round, fill);
            if failed {
                let c = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
                painter.extend(egui::Shape::dashed_line(&c, s, 5.0, 3.0));
            } else {
                painter.rect_stroke(rect, round, s);
            }
            match it.role {
                Role::Hub => {
                    painter.text(Pos2::new(rect.left() + 8.0 * z, p.y), Align2::LEFT_CENTER, &part.id, font.clone(), ink);
                    if labels {
                        painter.text(Pos2::new(rect.right() - 8.0 * z, p.y), Align2::RIGHT_CENTER, text, small.clone(), faint);
                    }
                }
                _ => {
                    painter.text(p, Align2::CENTER_CENTER, &part.id, small.clone(), ink);
                    if labels && !text.is_empty() {
                        let (y, al) = if it.role == Role::Source { (rect.bottom() + 3.0, Align2::CENTER_TOP) } else { (rect.top() - 3.0, Align2::CENTER_BOTTOM) };
                        painter.text(Pos2::new(p.x, y), al, text, small.clone(), faint);
                    }
                }
            }
            if resp.hover_pos().is_some_and(|h| rect.expand(4.0).contains(h)) {
                hit = Some(it.part);
            }
        }
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
            self.set(&n, v);
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
        let th = self.theme;
        for p in engines {
            ui.heading(&p.id);
            ui.label(format!("{}.{}", p.domain, p.kind));
            if p.kind == "gas_turbine" {
                let params = easyacft_parts::engine::Params::from_part(&p, &base);
                let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width().min(1100.0), 290.0), Sense::hover());
                let painter = ui.painter_at(rect);
                painter.rect_filled(rect, 0.0, th.paper);
                let id = p.id.clone();
                let get = |n: &str| self.get(&format!("{id}.{n}"));
                engine_view::draw(&painter, rect, &params, &get, th.ink, th.faint, th.fill);
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
            ui.separator();
        }
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
                    if ui.button("Fit").clicked() {
                        self.fit = true;
                        self.zoomed = false;
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
        if !self.report.is_empty() {
            egui::TopBottomPanel::bottom("report").resizable(true).show(ctx, |ui| {
                egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| {
                    ui.monospace(&self.report);
                });
            });
        }
        egui::SidePanel::left("left").default_width(220.0).show(ctx, |ui| self.left(ui));
        egui::SidePanel::right("right").default_width(360.0).max_width(460.0).show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui));
        });
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
    let mut v = egui::Visuals::light();
    let grey = Color32::from_gray(215);
    v.selection.bg_fill = grey;
    v.selection.stroke = Stroke::new(1.0f32, Color32::from_gray(20));
    v.hyperlink_color = Color32::from_gray(20);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0f32, Color32::from_gray(120));
    v.widgets.active.bg_stroke = Stroke::new(1.0f32, Color32::from_gray(40));
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

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--svg") {
        std::process::exit(if export(&args[1..]) { 0 } else { 1 });
    }
    let dir = args.first().cloned().unwrap_or_default();
    let flag = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let shot = flag("--shot");
    let tab = flag("--tab");
    let domain = flag("--domain");
    let select = flag("--select");
    let scenario = flag("--scenario");
    let run: f64 = flag("--run").and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let sets: Vec<(String, f64)> = args.windows(2).filter(|w| w[0] == "--set").filter_map(|w| w[1].split_once('=').map(|(n, v)| (n.to_owned(), v.parse().unwrap_or(0.0)))).collect();
    let mut viewport = egui::ViewportBuilder::default().with_inner_size([1400.0, 860.0]).with_title("EasyAcft Studio");
    if shot.is_some() {
        viewport = viewport.with_position([-20000.0, -20000.0]).with_taskbar(false).with_decorations(false);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "EasyAcft Studio",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_theme(egui::ThemePreference::Light);
            cc.egui_ctx.set_visuals_of(egui::Theme::Light, visuals());
            let mut app = App::new(dir);
            for (n, v) in &sets {
                app.set(n, *v);
            }
            if let (Some(a), Some(f)) = (&mut app.aircraft, &scenario) {
                if let Err(e) = std::fs::read_to_string(f).map_err(|e| e.to_string()).and_then(|t| easyacft_parts::scenario::run(a, &t, 0.05)) {
                    app.status = e;
                }
            }
            if let Some(a) = &mut app.aircraft {
                let mut t = 0.0;
                while t < run {
                    a.step(0.05);
                    t += 0.05;
                }
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
