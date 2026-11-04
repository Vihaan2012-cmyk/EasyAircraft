//! Meridional section of a gas turbine built from its own definition: flow path from mean radius and annulus
//! area, blade rows from pressure ratios, spools and bearings from the shaft count, gas temperatures, advected
//! airflow, ARP 755 stations. Every component is a failure target.

use crate::style::{self, State};
use easyacft_parts::engine::{Health, Params};
use eframe::egui::{epaint::Mesh, Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};

#[derive(Clone, Default)]
pub struct Live {
    pub id: String,
    /// LP, IP, HP speed, percent.
    pub n: [f64; 3],
    pub running: bool,
    pub failed: bool,
    pub health: Health,
    pub t0: f64,
    pub t3: f64,
    pub p3: f64,
    pub tet: f64,
    pub egt: f64,
    pub fuel: f64,
    pub thrust: f64,
    pub airflow: f64,
    pub shaft_power: f64,
    pub starter: bool,
    pub ignition: bool,
    pub eec: State,
    pub eec_id: Option<String>,
    pub reverser: f64,
    pub fire: State,
    pub fire_switch: Option<String>,
    pub gens: Vec<(String, State)>,
    pub pumps: Vec<(String, State)>,
    pub bleeds: Vec<(String, State)>,
    pub anti_ice: Option<(String, State)>,
    pub selected: Option<String>,
    pub time: f64,
}

/// A clickable component: `path` is the value a failure sets, `level` its current value.
#[derive(Clone, Debug)]
pub struct Hit {
    pub rect: Rect,
    pub path: String,
    pub label: String,
    pub level: f64,
    /// Damage takes 0 to 1; otherwise an on/off flag.
    pub graded: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Fan,
    Comp,
    Turb,
}

struct Sec {
    name: &'static str,
    kind: Kind,
    spool: usize,
    slot: &'static str,
    damage: f64,
    x0: f32,
    x1: f32,
    stages: usize,
}

struct Row {
    kind: Kind,
    spool: usize,
    x: f32,
    chord: f32,
    rotor: bool,
    damage: f64,
}

const BLADE: Color32 = Color32::from_rgb(205, 212, 220);
const VANE: Color32 = Color32::from_rgb(132, 142, 154);
const DISC: Color32 = Color32::from_rgb(78, 86, 96);
const CASE: Color32 = Color32::from_rgb(58, 62, 68);
const EDGE: Color32 = Color32::from_rgb(150, 160, 172);
const CALLOUT: Color32 = Color32::from_rgb(196, 202, 210);
const LEADER: Color32 = Color32::from_rgba_premultiplied(70, 72, 76, 110);
const BYPASS: Color32 = Color32::from_rgb(110, 215, 255);
const FUEL: Color32 = Color32::from_rgb(255, 157, 226);

fn stages_for(pr: f64, per: f64) -> usize {
    ((pr.max(1.0).ln() / per.ln()).ceil() as usize).clamp(1, 14)
}

/// Gas temperature (K) to colour.
pub fn heat(t: f64) -> Color32 {
    let stops = [(230.0, [70, 120, 200]), (330.0, [90, 190, 255]), (500.0, [120, 220, 190]), (700.0, [255, 214, 100]), (1000.0, [255, 140, 40]), (1350.0, [255, 70, 30]), (1750.0, [255, 230, 210])];
    let t = t.clamp(stops[0].0, stops[stops.len() - 1].0);
    for w in stops.windows(2) {
        if t <= w[1].0 {
            let k = ((t - w[0].0) / (w[1].0 - w[0].0)) as f32;
            let c = |i: usize| (w[0].1[i] as f32 + k * (w[1].1[i] as f32 - w[0].1[i] as f32)) as u8;
            return Color32::from_rgb(c(0), c(1), c(2));
        }
    }
    Color32::WHITE
}

fn damaged(base: Color32, d: f64) -> Color32 {
    if d <= 0.0 {
        return base;
    }
    let to = if d >= 0.5 { style::RED } else { style::ORANGE };
    let k = (0.55 + 0.45 * d.min(1.0)) as f32;
    let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k) as u8;
    Color32::from_rgb(m(base.r(), to.r()), m(base.g(), to.g()), m(base.b(), to.b()))
}

/// Monotone cubic through control points.
fn smooth(pts: &[(f32, f32)], x: f32) -> f32 {
    if x <= pts[0].0 {
        return pts[0].1;
    }
    let slope = |k: usize| -> f32 {
        if k == 0 || k == pts.len() - 1 {
            return 0.0;
        }
        let d0 = (pts[k].1 - pts[k - 1].1) / (pts[k].0 - pts[k - 1].0).max(1e-3);
        let d1 = (pts[k + 1].1 - pts[k].1) / (pts[k + 1].0 - pts[k].0).max(1e-3);
        if d0 * d1 <= 0.0 { 0.0 } else { 2.0 / (1.0 / d0 + 1.0 / d1) }
    };
    for i in 0..pts.len() - 1 {
        let ((x0, y0), (x1, y1)) = (pts[i], pts[i + 1]);
        if x <= x1 {
            let h = (x1 - x0).max(1e-3);
            let t = ((x - x0) / h).clamp(0.0, 1.0);
            let (t2, t3) = (t * t, t * t * t);
            return (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * slope(i) * h + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * slope(i + 1) * h;
        }
    }
    pts[pts.len() - 1].1
}

/// Puts labels of the given centre and width into the fewest rows without overlap; returns each label's row.
fn rows_for(items: &[(f32, f32)], blocked: &[(f32, f32)], max_rows: usize) -> Vec<Option<usize>> {
    let mut taken: Vec<Vec<(f32, f32)>> = vec![blocked.to_vec(); max_rows];
    items
        .iter()
        .map(|&(x, w)| {
            let (a, b) = (x - w / 2.0 - 3.0, x + w / 2.0 + 3.0);
            let row = (0..max_rows).find(|&r| !taken[r].iter().any(|&(l, h)| a < h && b > l))?;
            taken[row].push((a, b));
            Some(row)
        })
        .collect()
}

struct Pen<'a> {
    p: &'a Painter,
    o: Pos2,
    s: f32,
}

impl Pen<'_> {
    fn at(&self, x: f32, r: f32) -> Pos2 {
        self.o + Vec2::new(x * self.s, -r * self.s)
    }
    fn w(&self, w: f32) -> f32 {
        (w * self.s).max(0.7)
    }
    fn px(&self, size: f32) -> f32 {
        (size * self.s).max(7.5)
    }
    /// Width of a monospace label in drawing units.
    fn text_w(&self, t: &str, size: f32) -> f32 {
        t.chars().count() as f32 * self.px(size) * 0.62 / self.s
    }
    fn rect(&self, x0: f32, r0: f32, x1: f32, r1: f32) -> Rect {
        Rect::from_two_pos(self.at(x0, r0), self.at(x1, r1))
    }
    fn line(&self, pts: &[(f32, f32)], w: f32, c: Color32) {
        self.p.add(Shape::line(pts.iter().map(|&(x, r)| self.at(x, r)).collect(), Stroke::new(self.w(w), c)));
    }
    /// A horizontal run that hops over the vertical lines at `over`.
    fn hop_line(&self, x0: f32, x1: f32, r: f32, over: &[f32], w: f32, c: Color32) {
        let (lo, hi) = (x0.min(x1), x0.max(x1));
        let mut cur = lo;
        let mut cuts: Vec<f32> = over.iter().copied().filter(|&o| o > lo + 3.0 && o < hi - 3.0).collect();
        cuts.sort_by(f32::total_cmp);
        for o in cuts {
            self.line(&[(cur, r), (o - 3.0, r)], w, c);
            let arc: Vec<(f32, f32)> = (0..=6).map(|k| {
                let a = std::f32::consts::PI * k as f32 / 6.0;
                (o - 3.0 * a.cos(), r + 3.0 * a.sin())
            }).collect();
            self.line(&arc, w, c);
            cur = o + 3.0;
        }
        self.line(&[(cur, r), (hi, r)], w, c);
    }
    fn curve(&self, f: &dyn Fn(f32) -> f32, x0: f32, x1: f32, w: f32, c: Color32) {
        let n = (((x1 - x0) / 4.0).ceil() as usize).max(2);
        let pts: Vec<(f32, f32)> = (0..=n).map(|k| x0 + (x1 - x0) * k as f32 / n as f32).map(|x| (x, f(x))).collect();
        self.line(&pts, w, c);
    }
    fn poly(&self, pts: &[(f32, f32)], fill: Color32, w: f32, c: Color32) {
        let st = if w > 0.0 { Stroke::new(self.w(w), c) } else { Stroke::NONE };
        self.p.add(Shape::convex_polygon(pts.iter().map(|&(x, r)| self.at(x, r)).collect(), fill, st));
    }
    /// Region between two radius curves, coloured per column.
    fn fill(&self, top: &dyn Fn(f32) -> f32, bottom: &dyn Fn(f32) -> f32, x0: f32, x1: f32, colour: &dyn Fn(f32) -> Color32) {
        let n = (((x1 - x0) / 3.0).ceil() as usize).max(2);
        let mut mesh = Mesh::default();
        for k in 0..=n {
            let x = x0 + (x1 - x0) * k as f32 / n as f32;
            let c = colour(x);
            mesh.colored_vertex(self.at(x, top(x)), c);
            mesh.colored_vertex(self.at(x, bottom(x)), c);
            if k > 0 {
                let i = (2 * k) as u32;
                mesh.add_triangle(i - 2, i - 1, i);
                mesh.add_triangle(i - 1, i + 1, i);
            }
        }
        self.p.add(Shape::mesh(mesh));
    }
    fn circle(&self, x: f32, r: f32, rad: f32, fill: Color32, w: f32, c: Color32) {
        let st = if w > 0.0 { Stroke::new(self.w(w), c) } else { Stroke::NONE };
        self.p.circle(self.at(x, r), rad * self.s, fill, st);
    }
    fn text(&self, x: f32, r: f32, t: &str, size: f32, c: Color32, al: Align2) {
        self.p.text(self.at(x, r), al, t, FontId::monospace(self.px(size)), c);
    }
    /// Text shrunk to fit `max_w` units, centred on (x, r).
    fn text_in(&self, x: f32, r: f32, t: &str, size: f32, max_w: f32, c: Color32) {
        let k = (max_w / self.text_w(t, size).max(1.0)).min(1.0);
        let px = self.px(size) * k;
        if px >= 6.0 {
            self.p.text(self.at(x, r), Align2::CENTER_CENTER, t, FontId::monospace(px), c);
        }
    }
    fn tag(&self, rect: Rect, st: State, t: &str, size: f32) {
        style::squircle(self.p, rect, 5.0 * self.s, st);
        let w = rect.width() / self.s - 6.0;
        let c = rect.center();
        let k = (w / self.text_w(t, size).max(1.0)).min(1.0);
        let px = self.px(size) * k;
        if px >= 6.0 {
            self.p.text(c, Align2::CENTER_CENTER, t, FontId::monospace(px), style::text_of(st));
        }
    }
    /// Particles carried along a streamline at the local gas speed (units/s), each a streak as long as its speed.
    fn particles(&self, pts: &[(f32, f32)], speed: &dyn Fn(f32) -> f32, colour: &dyn Fn(f32) -> Color32, time: f32, count: usize, w: f32) {
        if pts.len() < 2 {
            return;
        }
        let mut tau = vec![0.0f32];
        for s in pts.windows(2) {
            let len = ((s[1].0 - s[0].0).powi(2) + (s[1].1 - s[0].1).powi(2)).sqrt();
            tau.push(tau.last().unwrap() + len / speed((s[0].0 + s[1].0) / 2.0).max(1.0));
        }
        let total = *tau.last().unwrap();
        for k in 0..count {
            let t = (time + k as f32 * total / count as f32).rem_euclid(total);
            let i = tau.partition_point(|&v| v <= t).clamp(1, pts.len() - 1);
            let u = (t - tau[i - 1]) / (tau[i] - tau[i - 1]).max(1e-6);
            let (a, b) = (pts[i - 1], pts[i]);
            let p = (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u);
            let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt().max(1e-3);
            let streak = (speed(p.0) * 0.05).clamp(3.0, 28.0);
            let back = (p.0 - (b.0 - a.0) / len * streak, p.1 - (b.1 - a.1) / len * streak);
            let c = colour(p.0);
            self.line(&[back, p], w, c.gamma_multiply(0.55));
            self.circle(p.0, p.1, w * 0.75, c, 0.0, Color32::TRANSPARENT);
        }
    }
}

pub fn draw(painter: &Painter, rect: Rect, title: &str, p: &Params, l: &Live) -> Vec<Hit> {
    painter.rect_filled(rect, 10.0, style::PANEL);
    let mut hits: Vec<Hit> = Vec::new();
    let path = |n: &str| format!("{}.{n}", l.id);
    let turboprop = p.design_power > 0.0;
    let fan = p.pr[0] > 1.0 && !turboprop;
    let bypass = fan && p.bpr > 0.05;
    let spools = p.spools.clamp(1, 3);
    let lit = l.running && !l.failed;
    let h = l.health;

    let (top_r, bottom_r) = (488.0f32, -96.0f32);
    let s = ((rect.width() - 20.0) / 1090.0).min((rect.height() - 16.0) / (top_r - bottom_r));
    let o = Pos2::new(rect.left() + 10.0 + 62.0 * s, rect.top() + 8.0 + top_r * s);
    let pen = Pen { p: painter, o, s };

    // Radial scheme: fan tip, hub and splitter from bypass ratio
    let r_tip: f32 = if fan { 300.0 } else if turboprop { 120.0 } else { 160.0 };
    let r_hub_fan = r_tip * 0.3;
    let k_bpr = 0.6 * p.bpr.max(0.0) as f32;
    let r_split = if bypass { ((r_tip * r_tip + k_bpr * (r_hub_fan + 20.0).powi(2)) / (1.0 + k_bpr)).sqrt() } else { r_tip };
    let rm_core = if bypass { (r_hub_fan + 20.0 + r_split) / 2.0 * 0.82 } else { r_tip * 0.62 };

    // Axial layout
    let lp = if spools == 1 { 2 } else { 0 };
    let mut comps: Vec<(&'static str, Kind, usize, usize, &'static str, f64, f64)> = Vec::new();
    if fan {
        comps.push(("FAN", Kind::Fan, lp, 1, "fan_damage", h.comp[0], p.pr[0]));
    } else if p.pr[0] > 1.0 {
        comps.push(("LP COMPRESSOR", Kind::Comp, lp, stages_for(p.pr[0], 1.25), "fan_damage", h.comp[0], p.pr[0]));
    }
    if p.pr[1] > 1.0 {
        comps.push((if spools == 3 { "LP COMPRESSOR" } else { "BOOSTER" }, Kind::Comp, lp, stages_for(p.pr[1], 1.2), "lpc_damage", h.comp[1], p.pr[1]));
    }
    if p.pr[2] > 1.0 {
        comps.push(("IP COMPRESSOR", Kind::Comp, if spools == 3 { 1 } else { lp }, stages_for(p.pr[2], 1.18), "ipc_damage", h.comp[2], p.pr[2]));
    }
    if p.pr[3] > 1.0 {
        comps.push(("HP COMPRESSOR", Kind::Comp, 2, stages_for(p.pr[3], 1.3), "hpc_damage", h.comp[3], p.pr[3]));
    }
    let lp_stages = if turboprop { 3 } else if spools >= 2 { ((2.0 + p.bpr.max(0.0) * 0.55).round() as usize).clamp(2, 8) } else { 0 };
    let mut turbs: Vec<(&'static str, usize, usize, &'static str, f64)> = vec![("HP TURBINE", 2, if p.pr[3] > 9.0 && spools < 3 { 2 } else { 1 }, "hpt_damage", h.turb[0])];
    if spools == 3 {
        turbs.push(("IP TURBINE", 1, 1, "ipt_damage", h.turb[1]));
    }
    if lp_stages > 0 {
        turbs.push((if turboprop { "POWER TURBINE" } else { "LP TURBINE" }, 0, lp_stages, "lpt_damage", h.turb[2]));
    }
    let row_w = |k: Kind| match k {
        Kind::Fan => 70.0,
        Kind::Comp => 15.0,
        Kind::Turb => 22.0,
    };
    let intake = if fan { 130.0 } else { 70.0 };
    let gaps = 18.0;
    let comb_len = 95.0;
    let raw: f32 = intake + comps.iter().map(|c| c.3 as f32 * row_w(c.1) + gaps).sum::<f32>() + comb_len + turbs.iter().map(|t| t.2 as f32 * row_w(Kind::Turb) + gaps).sum::<f32>();
    let sx = 780.0 / raw;
    let mut x = intake * sx;
    let mut secs: Vec<Sec> = Vec::new();
    let mut prs: Vec<f64> = Vec::new();
    for c in &comps {
        let w = (c.3 as f32 * row_w(c.1) + gaps) * sx;
        secs.push(Sec { name: c.0, kind: c.1, spool: c.2, slot: c.4, damage: c.5, x0: x, x1: x + w, stages: c.3 });
        prs.push(c.6);
        x += w;
    }
    let (comb0, comb1) = (x, x + comb_len * sx);
    x = comb1;
    for t in &turbs {
        let w = (t.2 as f32 * row_w(Kind::Turb) + gaps) * sx;
        secs.push(Sec { name: t.0, kind: Kind::Turb, spool: t.1, slot: t.3, damage: t.4, x0: x, x1: x + w, stages: t.2 });
        x += w;
    }
    let x_end = x;
    let core_x0 = secs.iter().find(|c| c.kind == Kind::Comp).map_or(comb0, |c| c.x0);
    let fan_x = secs.iter().find(|c| c.kind == Kind::Fan).map(|c| (c.x0, c.x1));
    let fan_x1 = fan_x.map_or(core_x0, |f| f.1);

    // Core flow path: mean radius and annulus height through each component, area falling with pressure ratio
    let mut mean: Vec<(f32, f32)> = Vec::new();
    let mut height: Vec<(f32, f32)> = Vec::new();
    let pi = std::f32::consts::PI;
    let mut area = if bypass { pi * (r_split.powi(2) - (r_hub_fan + 20.0).powi(2)) } else { pi * (r_tip * 0.9).powi(2) * 0.55 };
    let h_of = |a: f32, rm: f32| a / (2.0 * pi * rm);
    if let Some((f0, f1)) = fan_x {
        mean.push((f0, (r_hub_fan + r_split) / 2.0));
        height.push((f0, r_split - r_hub_fan));
        let rm = (r_hub_fan + 20.0 + r_split) / 2.0;
        mean.push((f1, rm));
        height.push((f1, h_of(area, rm)));
    } else {
        mean.push((0.0, rm_core));
        height.push((0.0, h_of(area, rm_core)));
    }
    let comp_idx: Vec<usize> = (0..secs.len()).filter(|&i| secs[i].kind == Kind::Comp).collect();
    let nc = comp_idx.len().max(1) as f32;
    for (j, &i) in comp_idx.iter().enumerate() {
        let c = &secs[i];
        let rm = rm_core * (1.0 - 0.18 * (j as f32 + 1.0) / nc);
        mean.push((c.x0 + 6.0, rm));
        height.push((c.x0 + 6.0, h_of(area, rm)));
        area *= (prs[i] as f32).powf(-0.74);
        mean.push((c.x1 - 4.0, rm));
        height.push((c.x1 - 4.0, h_of(area, rm)));
    }
    let rm_comb = rm_core * 0.92;
    mean.push((comb0 + 14.0, rm_comb));
    height.push((comb0 + 14.0, h_of(area * 2.2, rm_comb)));
    mean.push((comb1 - 18.0, rm_comb));
    height.push((comb1 - 18.0, h_of(area * 2.0, rm_comb)));
    let mut t_area = area * 1.35;
    for (j, t) in secs.iter().filter(|c| c.kind == Kind::Turb).enumerate() {
        let rm = rm_comb * (1.0 + 0.12 * j as f32);
        mean.push((t.x0 + 4.0, rm));
        height.push((t.x0 + 4.0, h_of(t_area, rm)));
        t_area *= match t.name {
            "HP TURBINE" => 1.9,
            "IP TURBINE" => 1.7,
            _ => 1.0 + 0.5 * t.stages as f32,
        };
        mean.push((t.x1 - 4.0, rm * 1.05));
        height.push((t.x1 - 4.0, h_of(t_area, rm * 1.05)));
    }
    let last_rm = mean.last().unwrap().1;
    mean.push((x_end + 40.0, last_rm * 0.92));
    height.push((x_end + 40.0, h_of(t_area * 0.9, last_rm * 0.92)));
    let tip_cap = if bypass { r_split - 20.0 } else { r_tip - 50.0 };
    let tip = |x: f32| (smooth(&mean, x) + smooth(&height, x) / 2.0).min(tip_cap.max(140.0));
    let hub = |x: f32| (smooth(&mean, x) - smooth(&height, x) / 2.0).max(26.0);

    // Gas temperature along the core
    let t0 = l.t0.max(200.0);
    let t3 = if l.t3 > 200.0 { l.t3 } else { t0 };
    let tet = if lit { l.tet.max(t3) } else { t3 };
    let egt = if lit { l.egt.max(t0) } else { t3 };
    let t_fan = t0 * p.pr[0].max(1.0).powf(0.286 / 0.9);
    let temp_at = |x: f32| -> f64 {
        if x < core_x0 {
            t0 + (t_fan - t0) * (x / core_x0.max(1.0)).clamp(0.0, 1.0) as f64
        } else if x < comb0 {
            t_fan + (t3 - t_fan) * ((x - core_x0) / (comb0 - core_x0).max(1.0)) as f64
        } else if x < comb1 {
            let k = ((x - comb0) / (comb1 - comb0)) as f64;
            if lit { t3 + (tet * 1.08 - t3) * (k * 1.6).min(1.0) - tet * 0.08 * ((k - 0.6).max(0.0) / 0.4) } else { t3 }
        } else if x < x_end {
            tet + (egt - tet) * ((x - comb1) / (x_end - comb1).max(1.0)).powf(0.7) as f64
        } else {
            egt
        }
    };
    // Axial gas speed relative to the compressor's: near-constant through compressors, slow in the diffuser and
    // combustor, accelerated by the nozzle guide vanes, fastest leaving the nozzle
    let core_rel = |x: f32| -> f32 {
        if x < core_x0 {
            0.9
        } else if x < comb0 {
            1.0
        } else if x < comb0 + 16.0 {
            1.0 - 0.65 * (x - comb0) / 16.0
        } else if x < comb1 - 8.0 {
            0.3
        } else if x < x_end {
            1.7 - 0.3 * (x - comb1) / (x_end - comb1).max(1.0)
        } else {
            2.3
        }
    };

    // Nacelle, bypass duct, core cowl
    let core_case = |x: f32| tip(x) + 10.0;
    let cowl = |x: f32| core_case(x) + if bypass { 26.0 } else { 18.0 };
    let nac_end = if bypass { core_x0 + (x_end - core_x0) * if p.bpr > 4.0 { 0.42 } else { 0.85 } } else { x_end };
    let lip = 12.0f32;
    let nac_in = |x: f32| -> f32 {
        if !bypass {
            return cowl(x) + 6.0;
        }
        if x < lip + 40.0 {
            r_tip * 1.02 + 18.0 * (1.0 - (x - lip) / 40.0).max(0.0).powi(2)
        } else if x < nac_end - 120.0 {
            r_tip + 4.0
        } else {
            r_tip + 4.0 - (x - (nac_end - 120.0)) / 120.0 * 24.0
        }
    };
    let nac_out = |x: f32| -> f32 {
        if !bypass {
            return cowl(x) + 24.0;
        }
        let t = ((x - lip) / (nac_end - lip)).clamp(0.0, 1.0);
        let shape = if t < 0.1 { (t / 0.1).sqrt() } else { 1.0 - ((t - 0.45).max(0.0) / 0.55).powi(2) * 0.85 };
        nac_in(x) + 6.0 + (r_tip + 52.0 - nac_in(x)) * shape.max(0.0)
    };
    let duct_in = |x: f32| if x < core_x0 - 10.0 { r_split + 6.0 } else { cowl(x.min(x_end)) };
    let outer_top = if bypass { r_tip + 52.0 } else { cowl(core_x0) + 24.0 };

    // Gas fields
    let warm = |x: f32| heat(temp_at(x)).gamma_multiply(0.36);
    pen.fill(&|x| tip(x), &|x| hub(x), core_x0 - 30.0, x_end + 40.0, &warm);
    if bypass {
        let c = heat(t_fan).gamma_multiply(0.24);
        pen.fill(&|x| nac_in(x.min(nac_end)), &|x| duct_in(x), fan_x1, nac_end, &|_| c);
        let inflow = heat(t0).gamma_multiply(0.18);
        pen.fill(&|x| nac_in(x), &|_| r_hub_fan, lip, fan_x.map_or(core_x0, |f| f.0), &|_| inflow);
    }
    let plume = |x: f32| heat(egt - (egt - t0) * 0.5 * ((x - x_end) / 160.0).clamp(0.0, 1.0) as f64).gamma_multiply(0.24);
    pen.fill(&|x| tip(x_end) * (1.0 - (x - x_end) / 900.0), &|x| (hub(x_end) * (1.0 - (x - x_end) / 120.0)).max(0.0), x_end, x_end + 120.0, &plume);

    // Casings
    pen.fill(&|x| core_case(x) + 6.0, &|x| core_case(x), core_x0 - 20.0, x_end, &|_| CASE);
    pen.curve(&|x| core_case(x), core_x0 - 20.0, x_end, 1.2, EDGE);
    pen.curve(&|x| core_case(x) + 6.0, core_x0 - 20.0, x_end, 1.2, EDGE);
    if bypass {
        pen.fill(&|x| cowl(x), &|x| core_case(x) + 6.0, core_x0 - 10.0, x_end + 10.0, &|_| Color32::from_rgb(42, 45, 50));
        pen.curve(&|x| cowl(x), core_x0 - 10.0, x_end + 10.0, 1.6, EDGE);
        pen.poly(&[(fan_x1 + 6.0, r_split), (core_x0 - 10.0, cowl(core_x0 - 10.0)), (core_x0 - 10.0, core_case(core_x0 - 10.0) + 6.0)], CASE, 1.2, EDGE);
        pen.fill(&|x| nac_out(x), &|x| nac_in(x), lip, nac_end, &|_| Color32::from_rgb(52, 56, 62));
        pen.curve(&|x| nac_out(x), lip, nac_end, 1.8, EDGE);
        pen.curve(&|x| nac_in(x), lip, nac_end, 1.4, EDGE);
        if let Some((f0, f1)) = fan_x {
            pen.fill(&|x| nac_in(x) + 14.0, &|x| nac_in(x), f0 - 30.0, f1 + 60.0, &|_| Color32::from_rgb(76, 82, 90));
            for k in 0..10 {
                let lx = f0 - 26.0 + k as f32 * 9.0;
                pen.line(&[(lx, nac_in(lx) + 2.0), (lx + 5.0, nac_in(lx) + 12.0)], 0.8, Color32::from_rgb(110, 118, 128));
            }
        }
        let rx = nac_end - 150.0;
        let open = l.reverser.clamp(0.0, 1.0) as f32;
        for k in 0..6 {
            let cx = rx + 8.0 + k as f32 * 8.0;
            pen.line(&[(cx, nac_in(cx) + 4.0), (cx + 6.0, nac_out(cx) - 6.0)], 1.0, if open > 0.0 { style::ORANGE } else { Color32::from_rgb(98, 106, 116) });
        }
        if open > 0.01 {
            let r0 = nac_in(rx + 60.0);
            pen.poly(&[(rx + 60.0, r0), (rx + 60.0 + 40.0 * open, cowl(rx + 60.0) + 4.0), (rx + 70.0 + 40.0 * open, cowl(rx + 60.0) + 4.0), (rx + 70.0, r0)], style::WARN_FILL, 1.6, style::ORANGE);
        }
    } else if turboprop {
        pen.fill(&|x| cowl(x) + 20.0, &|x| cowl(x), core_x0 - 40.0, x_end + 10.0, &|_| Color32::from_rgb(52, 56, 62));
        pen.curve(&|x| cowl(x) + 20.0, core_x0 - 40.0, x_end + 10.0, 1.6, EDGE);
    }
    hits.push(Hit { rect: pen.rect(core_x0, core_case(core_x0), x_end, core_case(x_end) + 6.0), path: path("case_leak"), label: "Core casing: hot-gas leak".into(), level: h.leak, graded: true });
    let hub_end = hub(x_end);
    let cone = |x: f32| hub_end * (1.0 - ((x - x_end) / 150.0).clamp(0.0, 1.0).powf(1.4));
    pen.fill(&|x| cone(x).max(0.5), &|_| 0.0, x_end - 4.0, x_end + 150.0, &|_| Color32::from_rgb(60, 64, 70));
    pen.curve(&cone, x_end - 4.0, x_end + 150.0, 1.4, EDGE);
    pen.line(&[(x_end, core_case(x_end)), (x_end + 40.0, tip(x_end) * 0.98)], 2.0, EDGE);

    // Blade rows, discs, drums
    let spool_pct = |sp: usize| if spools == 1 { l.n[2] } else { l.n[sp] };
    let shaft_r = |sp: usize| 9.0 + 6.0 * sp as f32;
    let mut rows: Vec<Row> = Vec::new();
    for c in &secs {
        let pitch = (c.x1 - c.x0 - 8.0) / c.stages as f32;
        for st in 0..c.stages {
            let bx = c.x0 + 4.0 + st as f32 * pitch;
            let d = c.damage;
            match c.kind {
                Kind::Fan => rows.push(Row { kind: Kind::Fan, spool: c.spool, x: bx, chord: pitch * 0.55, rotor: true, damage: d }),
                Kind::Comp => {
                    rows.push(Row { kind: Kind::Comp, spool: c.spool, x: bx + pitch * 0.08, chord: pitch * 0.4, rotor: true, damage: d });
                    rows.push(Row { kind: Kind::Comp, spool: c.spool, x: bx + pitch * 0.55, chord: pitch * 0.34, rotor: false, damage: d * 0.5 });
                }
                Kind::Turb => {
                    rows.push(Row { kind: Kind::Turb, spool: c.spool, x: bx + pitch * 0.04, chord: pitch * 0.38, rotor: false, damage: d * 0.5 });
                    rows.push(Row { kind: Kind::Turb, spool: c.spool, x: bx + pitch * 0.5, chord: pitch * 0.4, rotor: true, damage: d });
                }
            }
        }
    }
    let mut ogv_x = None;
    for (ri, r) in rows.iter().enumerate() {
        let turning = spool_pct(r.spool) > 2.0;
        let base = if r.rotor { BLADE } else { VANE };
        let base = if r.rotor && !turning { base.gamma_multiply(0.65) } else { base };
        let surging = h.surge && r.spool == 2 && r.kind == Kind::Comp;
        let c = damaged(if surging { style::ORANGE } else { base }, r.damage);
        match r.kind {
            Kind::Fan => {
                let (h0, t0r) = (r_hub_fan, r_tip - 3.0);
                let le = r.x;
                let broken = r.damage >= 0.5;
                let t_top = if broken { h0 + (t0r - h0) * 0.55 } else { t0r };
                let k = (t_top - h0) / (t0r - h0);
                pen.poly(&[(le - 8.0 * k, t_top), (le + r.chord + 10.0 * k, t_top - 4.0), (le + r.chord * 0.85, h0 + 4.0), (le + 6.0, h0)], c.gamma_multiply(0.9), 1.0, Color32::from_rgb(230, 236, 242));
                if broken {
                    for (dx, dr) in [(14.0, 30.0), (34.0, 18.0), (52.0, 40.0), (70.0, 24.0)] {
                        pen.poly(&[(le + dx, t_top + dr), (le + dx + 5.0, t_top + dr + 3.0), (le + dx + 2.0, t_top + dr - 4.0)], style::RED, 0.0, Color32::TRANSPARENT);
                    }
                } else {
                    for k in 1..4 {
                        let t = k as f32 / 4.0;
                        let a = (le - 8.0 + 14.0 * t, t0r - (t0r - h0) * t);
                        pen.line(&[a, (a.0 + r.chord * (0.95 - 0.2 * t), a.1 - 3.0)], 0.6, Color32::from_rgb(160, 168, 178));
                    }
                }
                let mut spin: Vec<(f32, f32)> = (0..=12).map(|i| {
                    let t = i as f32 / 12.0;
                    (le - 60.0 + 70.0 * t, r_hub_fan * (1.0 - (1.0 - t).powi(2)).sqrt())
                }).collect();
                spin.push((le + 10.0, 0.0));
                spin.push((le - 60.0, 0.0));
                pen.poly(&spin, Color32::from_rgb(70, 76, 84), 1.2, EDGE);
                pen.poly(&[(le + 6.0, h0), (le + r.chord, h0 + 2.0), (le + r.chord - 6.0, shaft_r(0) + 8.0), (le + 12.0, shaft_r(0) + 8.0)], DISC, 1.0, EDGE);
                if bypass {
                    let ox = le + r.chord + 40.0;
                    ogv_x = Some(ox + 9.0);
                    pen.poly(&[(ox, r_tip + 2.0), (ox + 22.0, r_tip + 2.0), (ox + 16.0, r_split + 6.0), (ox - 4.0, r_split + 6.0)], VANE, 1.0, EDGE);
                }
            }
            _ => {
                let (hb, tp) = (hub(r.x + r.chord / 2.0), tip(r.x + r.chord / 2.0));
                let lean = if r.rotor { 2.0 } else { -2.0 };
                let gap = if r.rotor { 1.5 } else { 0.0 };
                let tp = if r.rotor && r.damage >= 0.5 && ri % 2 == 0 { hb + (tp - hb) * 0.6 } else { tp };
                let root = if r.rotor { 0.0 } else { 2.0 };
                pen.poly(&[(r.x + lean, tp - gap), (r.x + r.chord + lean, tp - gap), (r.x + r.chord, hb + root), (r.x, hb + root)], c, 0.6, Color32::from_rgb(40, 44, 50));
                if r.rotor {
                    let bore = shaft_r(r.spool) + 5.0;
                    let xm = r.x + r.chord / 2.0;
                    let web = r.chord * 0.22;
                    let dc = damaged(DISC, r.damage * 0.6);
                    pen.poly(&[(r.x - 1.0, hb), (r.x + r.chord + 1.0, hb), (xm + web, hb - 8.0), (xm - web, hb - 8.0)], dc, 0.8, Color32::from_rgb(110, 120, 132));
                    if hb - 8.0 > bore + 6.0 {
                        pen.poly(&[(xm - web * 0.6, hb - 8.0), (xm + web * 0.6, hb - 8.0), (xm + web * 1.6, bore + 4.0), (xm - web * 1.6, bore + 4.0)], dc, 0.8, Color32::from_rgb(110, 120, 132));
                    }
                    if r.kind == Kind::Turb {
                        pen.line(&[(r.x + lean, tp - gap + 2.0), (r.x + r.chord + lean, tp - gap + 2.0)], 2.0, c);
                    }
                }
            }
        }
    }
    for c in &secs {
        let mx = (c.x0 + c.x1) / 2.0;
        let (r0, r1) = if c.kind == Kind::Fan { (r_hub_fan, r_tip) } else { (hub(mx) - 8.0, tip(mx)) };
        hits.push(Hit { rect: pen.rect(c.x0, r1, c.x1, r0), path: path(c.slot), label: format!("{}: blade and vane damage", style::label(c.name)), level: c.damage, graded: true });
    }

    // Combustor: diffuser, dome and swirler, liners with dilution holes, fuel injector, igniter, flame
    let liner_o = |x: f32| tip(x) - 4.0;
    let liner_i = |x: f32| hub(x) + 4.0;
    let comb_c = damaged(EDGE, h.burner);
    pen.curve(&liner_o, comb0 + 22.0, comb1 - 10.0, 2.0, comb_c);
    pen.curve(&liner_i, comb0 + 22.0, comb1 - 10.0, 2.0, comb_c);
    let dome_r = (liner_o(comb0 + 22.0) + liner_i(comb0 + 22.0)) / 2.0;
    pen.poly(&[(comb0 + 16.0, liner_o(comb0 + 22.0)), (comb0 + 24.0, liner_o(comb0 + 22.0)), (comb0 + 24.0, liner_i(comb0 + 22.0)), (comb0 + 16.0, liner_i(comb0 + 22.0))], Color32::from_rgb(90, 96, 104), 1.0, comb_c);
    pen.circle(comb0 + 24.0, dome_r, 4.0, Color32::from_rgb(120, 128, 138), 1.0, EDGE);
    for k in 0..3 {
        let hx = comb0 + 40.0 + k as f32 * (comb1 - comb0 - 60.0) / 3.0;
        pen.circle(hx, liner_o(hx), 1.8, style::PANEL, 0.8, EDGE);
        pen.circle(hx, liner_i(hx), 1.8, style::PANEL, 0.8, EDGE);
    }
    if h.burner >= 0.3 {
        let bx = comb0 + (comb1 - comb0) * 0.55;
        pen.circle(bx, liner_o(bx), 4.0, style::RED, 0.0, Color32::TRANSPARENT);
    }
    let fuel_c = if l.fuel > 0.0 { FUEL } else { style::GREY };
    let ig_c = if l.ignition { Color32::from_rgb(255, 211, 107) } else { style::GREY };
    pen.line(&[(comb0 + 40.0, core_case(comb0 + 40.0) + 6.0), (comb0 + 40.0, liner_o(comb0 + 40.0) - 2.0)], 2.4, ig_c);
    if lit {
        let ht = tip(comb0 + 40.0) - hub(comb0 + 40.0);
        for (rx, ry, a) in [(0.42f32, 0.42f32, 40u8), (0.32, 0.34, 90), (0.2, 0.22, 170)] {
            let w = comb1 - comb0;
            let pts: Vec<(f32, f32)> = (0..28).map(|i| {
                let t = i as f32 / 28.0 * std::f32::consts::TAU;
                (comb0 + 26.0 + w * rx * (1.0 + t.cos()), dome_r + ht * ry * t.sin())
            }).collect();
            pen.poly(&pts, Color32::from_rgba_unmultiplied(255, 110, 40, a), 0.0, Color32::TRANSPARENT);
        }
    }
    hits.push(Hit { rect: pen.rect(comb0, tip(comb0 + 30.0), comb1, hub(comb0 + 30.0)), path: path("combustor_damage"), label: "Combustor: liner and injector damage".into(), level: h.burner, graded: true });

    // Intermediate case strut (carries the radial drive) and turbine exhaust case
    let ix = secs.iter().find(|c| c.spool == 2 && c.kind == Kind::Comp).map_or(comb0 - 20.0, |c| c.x0 - 8.0);
    pen.poly(&[(ix - 8.0, core_case(ix)), (ix + 8.0, core_case(ix)), (ix + 4.0, hub(ix) - 2.0), (ix - 4.0, hub(ix) - 2.0)], CASE, 1.0, EDGE);
    pen.poly(&[(x_end - 14.0, core_case(x_end - 14.0)), (x_end + 8.0, core_case(x_end - 14.0)), (x_end + 4.0, hub(x_end) - 2.0), (x_end - 16.0, hub(x_end) - 2.0)], VANE.gamma_multiply(0.8), 1.0, EDGE);

    // Concentric shafts and bearings
    let mut speed_rows: Vec<(String, bool)> = Vec::new();
    for sp in 0..3 {
        let on_spool: Vec<&Sec> = secs.iter().filter(|c| c.spool == sp).collect();
        if on_spool.is_empty() {
            continue;
        }
        let x0 = on_spool.first().unwrap().x0 + 6.0;
        let x1 = on_spool.last().unwrap().x1 - 4.0;
        let r = shaft_r(sp);
        let seized = h.seized[sp];
        let col = if seized { style::RED } else if spool_pct(sp) > 2.0 { Color32::from_rgb(176, 188, 200) } else { Color32::from_rgb(108, 116, 124) };
        pen.fill(&|_| r + 2.5, &|_| r - 2.5, x0, x1, &|_| col.gamma_multiply(0.6));
        pen.line(&[(x0, r + 2.5), (x1, r + 2.5)], 0.8, col);
        let bearings: Vec<f32> = if sp == 0 { vec![x0 + 20.0, x1 + 2.0] } else { vec![x0 - 4.0, x1 + 2.0] };
        for bx in bearings {
            pen.poly(&[(bx - 4.0, r + 6.0), (bx + 4.0, r + 6.0), (bx + 4.0, r - 1.0), (bx - 4.0, r - 1.0)], Color32::from_rgb(40, 44, 50), 0.8, EDGE);
            pen.circle(bx, r + 2.5, 2.0, if seized { style::RED } else { Color32::from_rgb(210, 216, 224) }, 0.0, Color32::TRANSPARENT);
        }
        let name = match (spools, sp) {
            (1, _) => "N1",
            (2, 2) => "N2",
            (_, 0) => "N1",
            (_, 1) => "N2",
            _ => "N3",
        };
        let spool = ["LP", "IP", "HP"][sp];
        speed_rows.push((if seized { format!("{name:<4}{spool} SEIZED") } else { format!("{name:<4}{spool} {:>7.1} %", spool_pct(sp)) }, seized));
        hits.push(Hit { rect: pen.rect(x0, r + 4.0, x1, r - 4.0), path: path(&format!("seized_{}", spool.to_lowercase())), label: format!("{spool} shaft: bearing seizure"), level: seized as u8 as f64, graded: false });
    }
    let mut dash = -50.0;
    while dash < x_end + 170.0 {
        pen.line(&[(dash, 0.0), (dash + 14.0, 0.0)], 1.0, Color32::from_rgb(140, 146, 154));
        pen.line(&[(dash + 18.0, 0.0), (dash + 20.0, 0.0)], 1.0, Color32::from_rgb(140, 146, 154));
        dash += 24.0;
    }

    // Services: one corridor through the intermediate-case strut and the bifurcation carries the radial drive,
    // the merged bleed duct and the fuel supply; nothing else crosses the bypass
    let band_r = outer_top + 56.0;
    let core_n = l.n[2].max(l.n[1]);
    let (lane_d, lane_b, lane_f) = (ix, ix + 9.0, ix + 18.0);
    let bleed_r = |x: f32| core_case(x) + 12.0;
    let fuel_r = |x: f32| core_case(x) + if bypass { 19.0 } else { 15.0 };
    if bypass {
        let (fx0, fx1) = (ix - 9.0, ix + 27.0);
        let fr = |x: f32| nac_in(x.min(nac_end));
        if ix < nac_end {
            pen.fill(&fr, &|x| cowl(x), fx0, fx1, &|_| Color32::from_rgb(48, 51, 56));
            pen.line(&[(fx0, cowl(fx0)), (fx0, fr(fx0))], 1.0, EDGE);
            pen.line(&[(fx1, cowl(fx1)), (fx1, fr(fx1))], 1.0, EDGE);
        }
    }
    let drive_c = if core_n > 2.0 { Color32::from_rgb(176, 188, 200) } else { Color32::from_rgb(108, 116, 124) };
    pen.line(&[(lane_d, shaft_r(2) + 4.0), (lane_d, band_r - 12.0)], 2.4, drive_c);
    let any_bleed = l.bleeds.iter().map(|b| b.1).max().unwrap_or(State::Off);
    let comp_secs: Vec<&Sec> = secs.iter().filter(|c| c.kind == Kind::Comp).collect();
    let mut ports: Vec<(f32, String, State)> = Vec::new();
    for (i, (name, st)) in l.bleeds.iter().enumerate() {
        let sec = if i == 0 { comp_secs.last() } else { comp_secs.get(comp_secs.len().saturating_sub(2)) };
        let Some(sec) = sec else { continue };
        ports.push((sec.x0 + (sec.x1 - sec.x0) * if i == 0 { 0.6 } else { 0.75 }, name.clone(), *st));
    }
    for (bx, name, st) in &ports {
        let c = style::colour_of(*st);
        pen.line(&[(*bx, core_case(*bx) + 6.0), (*bx, bleed_r(*bx))], 2.0, c);
        pen.hop_line(*bx, lane_b, bleed_r(*bx), &[lane_d], 2.0, c);
        pen.circle(*bx, core_case(*bx) + 6.0, 2.2, c, 0.0, Color32::TRANSPARENT);
        let pr = pen.rect(bx - 6.0, bleed_r(*bx) + 4.0, bx + 6.0, core_case(*bx) + 4.0);
        hits.push(Hit { rect: pr, path: format!("{name}.failed"), label: format!("{}: bleed port failure", style::label(name)), level: (*st == State::Fail) as u8 as f64, graded: false });
    }
    let bc = style::colour_of(any_bleed);
    if !ports.is_empty() {
        pen.line(&[(lane_b, bleed_r(lane_b)), (lane_b, band_r - 18.0)], 2.2, bc);
    }
    pen.line(&[(lane_f, band_r - 24.0), (lane_f, fuel_r(lane_f))], 2.0, fuel_c);
    pen.line(&[(lane_f, fuel_r(lane_f)), (comb0 + 6.0, fuel_r(comb0 + 6.0))], 2.0, fuel_c);
    let dome = (comb0 + 22.0, dome_r);
    pen.line(&[(comb0 + 6.0, fuel_r(comb0 + 6.0)), (comb0 + 6.0, dome.1 + 6.0), dome], 2.2, fuel_c);

    // Accessory row: gearbox over the radial drive, precooler, FMU, EEC
    let mut boxes: Vec<(String, State, Option<String>)> = l.gens.iter().map(|(n, s)| (n.clone(), *s, Some(format!("{n}.failed")))).collect();
    boxes.push(("starter".into(), if l.starter { State::Live } else { State::Off }, None));
    boxes.push(("oil pump".into(), if core_n > 10.0 { State::Live } else { State::Off }, None));
    boxes.push(("fuel pump".into(), if l.fuel > 0.0 { State::Live } else { State::Off }, None));
    boxes.extend(l.pumps.iter().map(|(n, s)| (n.clone(), *s, Some(format!("{n}.failed")))));
    let bw = 62.0;
    let gb_w = boxes.len() as f32 * (bw + 4.0) + 4.0;
    let gb_x0 = (lane_d + 24.0 - gb_w).max(-40.0);
    let gb_x1 = gb_x0 + gb_w;
    pen.line(&[(lane_d, band_r - 12.0), (lane_d, band_r - 10.0)], 2.4, drive_c);
    style::squircle_with(painter, pen.rect(gb_x0, band_r - 2.0, gb_x1, band_r - 10.0), 3.0 * s, Color32::from_rgb(64, 69, 76), Stroke::new((1.0 * s).max(0.6), EDGE));
    for (i, (n, st, fail)) in boxes.iter().enumerate() {
        let bx = gb_x0 + 4.0 + i as f32 * (bw + 4.0);
        let r = pen.rect(bx, band_r + 20.0, bx + bw, band_r);
        pen.tag(r, *st, &style::label(n), 7.0);
        if let Some(f) = fail {
            hits.push(Hit { rect: r, path: f.clone(), label: format!("{}: failure", style::label(n)), level: (*st == State::Fail) as u8 as f64, graded: false });
        }
    }
    let pc_x = gb_x1.max(lane_f + 10.0) + 12.0;
    pen.line(&[(lane_b, band_r - 18.0), (pc_x + 14.0, band_r - 18.0), (pc_x + 14.0, band_r)], 2.2, bc);
    pen.tag(pen.rect(pc_x, band_r + 20.0, pc_x + 86.0, band_r), any_bleed, "PRECOOLER", 7.5);
    let fmu_x = pc_x + 98.0;
    let fmu_st = if l.fuel > 0.0 { State::Live } else { State::Off };
    pen.line(&[(lane_f, band_r - 24.0), (fmu_x + 14.0, band_r - 24.0), (fmu_x + 14.0, band_r)], 2.0, fuel_c);
    pen.tag(pen.rect(fmu_x, band_r + 20.0, fmu_x + 48.0, band_r), fmu_st, "FMU", 7.5);
    pen.text(fmu_x + 54.0, band_r - 22.0, &format!("{:.3} kg/s", l.fuel), 7.0, fuel_c, Align2::LEFT_BOTTOM);
    let eec_x = fmu_x + 60.0;
    let eec_r = pen.rect(eec_x, band_r + 20.0, eec_x + 52.0, band_r);
    pen.tag(eec_r, l.eec, "EEC", 7.5);
    if let Some(id) = &l.eec_id {
        hits.push(Hit { rect: eec_r, path: format!("{id}.failed"), label: "EEC / FADEC: failure".into(), level: (l.eec == State::Fail) as u8 as f64, graded: false });
    }
    let mut next_x = eec_x + 64.0;
    if let Some(sw) = &l.fire_switch {
        let st = if l.fire == State::Fail { State::Fail } else { State::Off };
        let fr = pen.rect(next_x, band_r + 20.0, next_x + 70.0, band_r);
        pen.tag(fr, st, "FIRE ZONE", 7.5);
        hits.push(Hit { rect: fr, path: format!("{sw}.on"), label: "Engine fire".into(), level: (l.fire == State::Fail) as u8 as f64, graded: false });
        next_x += 82.0;
    }
    let _ = next_x;
    if let Some((id, ai)) = &l.anti_ice {
        if bypass {
            let c = style::colour_of(*ai);
            let wall = |x: f32| (nac_in(x) + 14.0 + nac_out(x)) / 2.0 + 2.0;
            let x1 = (ix - 12.0).min(nac_end - 20.0);
            pen.curve(&wall, lip + 6.0, x1, 1.6, c);
            pen.circle(lip + 6.0, wall(lip + 6.0), 2.2, c, 0.0, Color32::TRANSPARENT);
            hits.push(Hit { rect: pen.rect(lip, wall(lip + 30.0) + 8.0, x1, wall(lip + 30.0) - 8.0), path: format!("{id}.failed"), label: "Nacelle anti-ice: failure".into(), level: (*ai == State::Fail) as u8 as f64, graded: false });
        }
    }
    if l.fire == State::Fail {
        pen.fill(&|x| cowl(x), &|x| core_case(x) + 6.0, core_x0, x_end, &|_| Color32::from_rgba_unmultiplied(255, 61, 99, 80));
    }

    // Airflow: particles advected at the local gas speed
    let flow = (l.airflow / p.airflow.max(1.0)) as f32;
    let time = l.time as f32;
    let base = 240.0 * flow;
    let moving = flow > 0.01;
    let core_flow = base * if h.surge { 0.45 } else { 1.0 };
    let core_lines = [0.12f32, 0.32, 0.5, 0.68, 0.88];
    for (k, frac) in core_lines.iter().enumerate() {
        let mut pts = Vec::new();
        let mut xx = core_x0 - 40.0;
        while xx < x_end + 140.0 {
            let xc = xx.min(x_end + 30.0);
            let (hb, tp) = (hub(xc), tip(xc));
            let y = if xx > x_end { hb * (1.0 - (xx - x_end) / 220.0) + (tp - hb) * frac } else { hb + (tp - hb) * frac };
            pts.push((xx, y));
            xx += 6.0;
        }
        if moving {
            pen.particles(&pts, &|x| core_rel(x) * core_flow, &|x| heat(temp_at(x)), time + k as f32 * 0.21, 26, 1.8);
        } else {
            pen.line(&pts, 0.6, Color32::from_rgba_unmultiplied(150, 170, 190, 40));
        }
    }
    let fan_face = fan_x.map_or(core_x0 - 40.0, |f| f.0);
    let intake_lines: Vec<f32> = if bypass { vec![0.15, 0.35, 0.55, 0.75, 0.92] } else { vec![0.3, 0.7] };
    for (k, frac) in intake_lines.iter().enumerate() {
        let r_in = r_hub_fan + 10.0 + (r_tip - r_hub_fan - 14.0) * frac;
        let mut pts = vec![(-50.0, r_in * 1.08), (lip, r_in)];
        let core_bound = (r_split - r_hub_fan) / (r_tip - r_hub_fan);
        if bypass && *frac > core_bound {
            let f = (frac - core_bound) / (1.0 - core_bound);
            let mut xx = fan_face;
            while xx < nac_end + 150.0 {
                let inner = duct_in(xx);
                let outer = nac_in(xx.min(nac_end));
                pts.push((xx, inner + (outer - inner) * (0.1 + 0.8 * f) - if xx > nac_end { (xx - nac_end) * 0.12 } else { 0.0 }));
                xx += 8.0;
            }
            let spd = |x: f32| base * if x < fan_x1 { 0.9 } else if x < nac_end { 1.15 } else { 1.6 } * (1.0 - 0.35 * h.comp[0] as f32);
            if moving {
                pen.particles(&pts, &spd, &|_| BYPASS, time + k as f32 * 0.37, 30, 1.8);
            } else {
                pen.line(&pts, 0.6, Color32::from_rgba_unmultiplied(150, 170, 190, 40));
            }
        } else {
            let into = (core_x0 - 40.0, hub(core_x0 - 40.0) + (tip(core_x0 - 40.0) - hub(core_x0 - 40.0)) * 0.5);
            pts.push((fan_face, r_in.min(r_split - 6.0)));
            pts.push(into);
            if moving {
                pen.particles(&pts, &|_| base * 0.9, &|_| heat(t0), time + k as f32 * 0.29, 12, 1.8);
            }
        }
    }
    if h.surge {
        if let Some(hpc) = secs.iter().find(|c| c.name == "HP COMPRESSOR") {
            for k in 0..5 {
                let r = hub(hpc.x0) + (tip(hpc.x0) - hub(hpc.x0)) * (k as f32 + 0.5) / 5.0;
                let ph = (time * 3.0 + k as f32 * 0.37).fract();
                let xs = hpc.x0 + 20.0 - ph * 60.0;
                pen.line(&[(xs + 12.0, r), (xs, r)], 2.0, style::ORANGE.gamma_multiply(1.0 - ph));
            }
        }
    }
    if h.leak > 0.0 {
        let lx = secs.iter().find(|c| c.name == "HP COMPRESSOR").map_or(comb0, |c| c.x1 - 6.0);
        for k in 0..6 {
            let ph = (time * 2.0 + k as f32 / 6.0).fract();
            let r = cowl(lx) + ph * 60.0 * h.leak as f32;
            pen.line(&[(lx + (k as f32 - 2.5) * 3.0, r), (lx + (k as f32 - 2.5) * 5.0, r + 8.0)], 2.0, heat(t3).gamma_multiply(1.0 - ph));
        }
    }

    // Stations (ARP 755) in a row under the axis
    let mut stations: Vec<(&str, f32, f64, Option<f64>)> = vec![("0", -30.0, t0, None), ("2", fan_face - 4.0, t0, None)];
    if bypass {
        stations.push(("13", fan_x1 + 30.0, t_fan, None));
        stations.push(("18", nac_end - 6.0, t_fan, None));
    }
    if let Some(c) = secs.iter().find(|c| c.kind == Kind::Comp) {
        stations.push(("21", c.x0, t_fan, None));
    }
    if let Some(c) = secs.iter().find(|c| c.name == "HP COMPRESSOR") {
        stations.push(("25", c.x0, temp_at(c.x0), None));
    }
    stations.push(("3", comb0 + 2.0, t3, Some(l.p3)));
    stations.push(("4", comb1 - 4.0, tet, Some(l.p3 * 0.95)));
    if let Some(c) = secs.iter().find(|c| c.name == "IP TURBINE") {
        stations.push(("44", c.x0, temp_at(c.x0), None));
    }
    if let Some(c) = secs.iter().find(|c| c.spool == 0 && c.kind == Kind::Turb) {
        stations.push(("45", c.x0, temp_at(c.x0), None));
    }
    stations.push(("5", x_end - 2.0, egt, None));
    stations.push(("8", x_end + 40.0, egt, None));
    let mut order: Vec<usize> = (0..stations.len()).collect();
    order.sort_by(|&a, &b| stations[a].1.total_cmp(&stations[b].1));
    let srow = rows_for(&order.iter().map(|&i| (stations[i].1, 16.0)).collect::<Vec<_>>(), &[], 2);
    for (k, &i) in order.iter().enumerate() {
        let (name, sx_, _, _) = stations[i];
        let r = -14.0 - 15.0 * srow[k].unwrap_or(1) as f32;
        pen.line(&[(sx_, -4.0), (sx_, r + 7.0)], 0.6, LEADER);
        pen.circle(sx_, r, 7.0, style::FILL, 1.0, CALLOUT);
        pen.text_in(sx_, r, name, 7.0, 12.0, CALLOUT);
    }

    // Component brackets under the stations
    let mut brackets: Vec<(f32, f32, String, f64)> = secs.iter().map(|c| (c.x0, c.x1, if c.kind == Kind::Fan { "FAN".to_owned() } else { format!("{} {} ST", c.name, c.stages) }, c.damage)).collect();
    brackets.push((comb0, comb1, "COMBUSTOR".into(), h.burner));
    brackets.sort_by(|a, b| a.0.total_cmp(&b.0));
    let brow = rows_for(&brackets.iter().map(|b| ((b.0 + b.1) / 2.0, pen.text_w(&b.2, 7.5).max(b.1 - b.0 - 4.0))).collect::<Vec<_>>(), &[], 3);
    for (k, (x0, x1, name, d)) in brackets.iter().enumerate() {
        let r = -50.0 - 13.0 * brow[k].unwrap_or(2) as f32;
        let c = if *d > 0.0 { damaged(CALLOUT, *d) } else { CALLOUT };
        pen.line(&[(x0 + 2.0, r + 4.0), (x0 + 2.0, r), (x1 - 2.0, r), (x1 - 2.0, r + 4.0)], 0.8, LEADER);
        let tw = pen.text_w(name, 7.5) + 6.0;
        let mx = (x0 + x1) / 2.0;
        painter.rect_filled(pen.rect(mx - tw / 2.0, r + 5.0, mx + tw / 2.0, r - 5.0), 2.0, style::PANEL);
        pen.text(mx, r, name, 7.5, c, Align2::CENTER_CENTER);
    }

    // Outer callouts above the nacelle, kept clear of the service corridor
    let mut outer: Vec<(f32, f32, &str)> = Vec::new();
    if bypass {
        if let Some((f0, f1)) = fan_x {
            outer.push(((f0 + f1) / 2.0, nac_in(f1) + 14.0, "FAN CASE"));
        }
        if let Some(ox) = ogv_x {
            outer.push((ox, r_tip, "OGV"));
        }
        outer.push((fan_x1 + 10.0, r_split, "SPLITTER"));
        outer.push((nac_end - 124.0, nac_out(nac_end - 124.0), "REVERSER"));
        if l.anti_ice.is_some() {
            outer.push((lip + 30.0, nac_out(lip + 30.0), "ANTI-ICE"));
        }
    }
    outer.sort_by(|a, b| a.0.total_cmp(&b.0));
    let blocked = [(ix - 12.0, ix + 30.0)];
    let orow = rows_for(&outer.iter().map(|o| (o.0, pen.text_w(o.2, 7.0))).collect::<Vec<_>>(), &blocked, 2);
    for (k, (tx, tr, name)) in outer.iter().enumerate() {
        let Some(row) = orow[k] else { continue };
        let r = outer_top + 12.0 + 12.0 * row as f32;
        pen.line(&[(*tx, r - 4.0), (*tx, *tr)], 0.6, LEADER);
        pen.text(*tx, r, name, 7.0, style::GREY_TEXT, Align2::CENTER_CENTER);
    }
    pen.line(&[(x_end + 80.0, -24.0), (x_end + 80.0, cone(x_end + 80.0) * 0.5)], 0.6, LEADER);
    pen.text(x_end + 80.0, -30.0, "EXHAUST CONE", 7.0, style::GREY_TEXT, Align2::CENTER_CENTER);

    // Station table and spool speeds at the right
    let tx = x_end + 92.0;
    let mut tr = band_r - 40.0;
    pen.text(tx, tr, "STN   T °C  P kPa", 7.5, CALLOUT, Align2::LEFT_TOP);
    for (name, _, t, pr) in &stations {
        tr -= 11.0;
        let pv = pr.map_or("-".to_owned(), |v| format!("{:.0}", v / 1e3));
        pen.text(tx, tr, &format!("{name:<4}{:>6.0} {pv:>6}", t - 273.15), 7.5, style::GREY_TEXT, Align2::LEFT_TOP);
    }
    tr -= 15.0;
    pen.text(tx, tr, "SPOOL SPEED", 7.5, CALLOUT, Align2::LEFT_TOP);
    for (row, seized) in &speed_rows {
        tr -= 11.0;
        pen.text(tx, tr, row, 7.5, if *seized { style::RED } else { style::GREY_TEXT }, Align2::LEFT_TOP);
    }

    // Title, state, active failures
    let edge = if l.failed { State::Fail } else if lit { State::Live } else { State::Off };
    pen.text(-50.0, top_r - 4.0, title, 13.0, style::text_of(edge), Align2::LEFT_TOP);
    let state = if l.failed { "FAILED" } else if lit { "RUNNING" } else if core_n > 2.0 { "TURNING" } else { "STOPPED, NO AIRFLOW" };
    let shp = if turboprop { format!("  SHAFT {:.0} kW", l.shaft_power / 1e3) } else { String::new() };
    pen.text(1010.0, top_r - 4.0, &format!("{state}{shp}  THRUST {:.1} kN  FF {:.3} kg/s  EGT {:.0} °C  W {:.0} kg/s  BPR {:.1}  OPR {:.1}", l.thrust / 1e3, l.fuel, l.egt - 273.15, l.airflow, p.bpr, p.pr.iter().product::<f64>()), 8.0, style::GREY_TEXT, Align2::RIGHT_TOP);
    let mut faults: Vec<String> = Vec::new();
    for c in secs.iter().filter(|c| c.damage > 0.0) {
        faults.push(format!("{} {:.0} %", c.name, c.damage * 100.0));
    }
    if h.comp[0] >= 0.5 && fan {
        faults.push("FAN BLADE OFF".into());
    }
    if h.burner > 0.0 {
        faults.push(format!("COMBUSTOR {:.0} %", h.burner * 100.0));
    }
    if h.surge {
        faults.push("SURGE".into());
    }
    if h.leak > 0.0 {
        faults.push("CASE LEAK".into());
    }
    for (sp, n) in ["LP", "IP", "HP"].iter().enumerate() {
        if h.seized[sp] {
            faults.push(format!("{n} SHAFT SEIZED"));
        }
    }
    if l.fire == State::Fail {
        faults.push("FIRE".into());
    }
    if !faults.is_empty() {
        pen.text(-50.0, top_r - 22.0, &faults.join("   "), 8.0, style::RED, Align2::LEFT_TOP);
    }
    let whole = pen.rect(-50.0, top_r - 2.0, 150.0, top_r - 20.0);
    hits.push(Hit { rect: whole, path: path("failed"), label: "Whole engine: failure".into(), level: l.failed as u8 as f64, graded: false });

    if let Some(sel) = &l.selected {
        for hh in hits.iter().filter(|hh| &hh.path == sel) {
            painter.rect_stroke(hh.rect.expand(2.0), 4.0, Stroke::new(1.6f32, style::SELECT));
        }
    }
    hits
}
