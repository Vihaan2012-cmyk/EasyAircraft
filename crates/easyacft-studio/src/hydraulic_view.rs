//! The hydraulic page: each circuit as a schematic from its reservoir through its pumps and their shutoff valves,
//! the pressure manifold, accumulators and priority valves, out to every user it powers (surfaces actuator by
//! actuator, the stabiliser and high-lift motors, gear legs with doors and bogies, brakes, steering, lines).

use crate::style::{self, State};
use easyacft_lang::Checked;
use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Vec2};

pub struct Hit {
    pub rect: Rect,
    pub part: usize,
}

const PA_PER_PSI: f64 = 6894.757;

/// What one circuit powers, by group.
#[derive(Default)]
struct Users {
    surfaces: Vec<usize>,
    motors: Vec<usize>,
    legs: Vec<usize>,
    brakes: Vec<usize>,
    steering: Vec<usize>,
    lines: Vec<usize>,
}

fn on_circuit(c: &Checked, part: usize, circuit: usize) -> bool {
    let p = &c.parts[part];
    p.part("circuit") == Some(circuit) || p.part("alternate_circuit") == Some(circuit)
}

fn users_of(c: &Checked, circuit: usize) -> Users {
    let mut u = Users::default();
    for (i, p) in c.parts.iter().enumerate() {
        let any = |field: &str| p.parts(field).iter().any(|&a| on_circuit(c, a, circuit)) || p.part(field).is_some_and(|a| on_circuit(c, a, circuit));
        match (p.domain, p.kind) {
            ("flight_control", "surface") if any("actuators") => u.surfaces.push(i),
            ("flight_control", "stabilizer" | "high_lift") if any("motors") => u.motors.push(i),
            ("landing_gear", "leg") if any("actuator") => u.legs.push(i),
            ("brakes", "wheel") if on_circuit(c, i, circuit) => u.brakes.push(i),
            ("steering", "unit") if any("actuators") => u.steering.push(i),
            ("hydraulic", "consumer") if on_circuit(c, i, circuit) => u.lines.push(i),
            _ => {}
        }
    }
    u
}

struct Pen<'a> {
    p: &'a Painter,
    o: Pos2,
    s: f32,
    hits: Vec<Hit>,
    selected: Option<usize>,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Pos2 {
        self.o + Vec2::new(x, y) * self.s
    }
    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(self.at(x, y), Vec2::new(w, h) * self.s)
    }
    fn text(&self, x: f32, y: f32, t: &str, size: f32, colour: Color32, align: Align2) {
        self.p.text(self.at(x, y), align, t, FontId::monospace(size * self.s), colour);
    }
    fn line(&self, pts: &[(f32, f32)], live: State) {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| self.at(x, y)).collect();
        self.p.add(eframe::egui::Shape::line(pts, style::stroke_of(live, self.s)));
    }
    /// A part box with a title and up to three value lines; clicking it selects the part.
    fn part(&mut self, part: usize, x: f32, y: f32, w: f32, h: f32, title: &str, lines: &[String], state: State) {
        let r = self.rect(x, y, w, h);
        style::squircle(self.p, r, 6.0 * self.s, state);
        if self.selected == Some(part) {
            style::squircle_with(self.p, r.expand(3.0 * self.s), 8.0 * self.s, Color32::TRANSPARENT, Stroke::new(2.0 * self.s, style::SELECT));
        }
        self.text(x + w / 2.0, y + 11.0, title, 10.5, style::text_of(state), Align2::CENTER_CENTER);
        for (k, l) in lines.iter().enumerate() {
            self.text(x + w / 2.0, y + 25.0 + 12.0 * k as f32, l, 9.5, style::GREY_TEXT, Align2::CENTER_CENTER);
        }
        self.hits.push(Hit { rect: r, part });
    }
}

fn psi(v: f64) -> String {
    format!("{:.0} psi", v / PA_PER_PSI)
}

/// Draws every circuit stacked; returns the boxes that can be clicked.
pub fn draw(painter: &Painter, rect: Rect, c: &Checked, get: &dyn Fn(&str) -> f64, selected: Option<usize>) -> Vec<Hit> {
    painter.rect_filled(rect, 10.0, style::PANEL);
    let circuits: Vec<usize> = (0..c.parts.len()).filter(|&i| c.parts[i].domain == "hydraulic" && c.parts[i].kind == "circuit").collect();
    if circuits.is_empty() {
        painter.text(rect.center(), Align2::CENTER_CENTER, "No hydraulic circuits", FontId::proportional(16.0), style::GREY_TEXT);
        return Vec::new();
    }
    let bands: Vec<Users> = circuits.iter().map(|&k| users_of(c, k)).collect();
    // Band height follows its longest user column.
    let rows = |u: &Users| u.surfaces.len().max(u.motors.len() + u.legs.len() + u.steering.len()).max(u.lines.len() + (u.brakes.len() + 3) / 4).max(6);
    let band_h: Vec<f32> = bands.iter().map(|u| 90.0 + 22.0 * rows(u) as f32).collect();
    let (w, h) = (1500.0f32, band_h.iter().sum::<f32>() + 12.0 * circuits.len() as f32);
    let s = ((rect.width() - 16.0) / w).min((rect.height() - 16.0) / h);
    let mut pen = Pen { p: painter, o: rect.min + Vec2::new(8.0, 8.0), s, hits: Vec::new(), selected };
    let g = |id: &str, out: &str| get(&format!("{id}.{out}"));
    let mut y0 = 0.0f32;
    for (n, &ci) in circuits.iter().enumerate() {
        let cp = &c.parts[ci];
        let id = cp.id.as_str();
        let bh = band_h[n];
        let pressure = g(id, "pressure");
        let live = if g(id, "failed") != 0.0 {
            State::Fail
        } else if g(id, "available") != 0.0 {
            State::Live
        } else if pressure > 50.0 * PA_PER_PSI {
            State::Warn
        } else {
            State::Off
        };
        style::squircle_with(painter, pen.rect(0.0, y0, w, bh), 10.0 * s, style::BG, Stroke::new(1.0_f32, style::GREY));
        pen.text(14.0, y0 + 16.0, &style::label(id), 15.0, style::text_of(live), Align2::LEFT_CENTER);
        pen.text(120.0, y0 + 16.0, &psi(pressure), 15.0, style::text_of(live), Align2::LEFT_CENTER);
        let flags: Vec<&str> = [("low", "LO PR"), ("low_level", "LO LEVEL"), ("overheat", "OVHT"), ("filter_clogged", "FILTER CLOG"), ("filter_bypass", "FILTER BYPASS")].iter().filter(|(o, _)| g(id, o) != 0.0).map(|(_, t)| *t).collect();
        pen.text(250.0, y0 + 16.0, &flags.join("  "), 12.0, style::ORANGE, Align2::LEFT_CENTER);

        // Reservoir
        let top = y0 + 36.0;
        let res_state = if g(id, "low_level") != 0.0 || g(id, "overheat") != 0.0 { State::Warn } else if g(id, "fluid") > 0.0 { State::Live } else { State::Off };
        pen.part(
            ci,
            12.0,
            top,
            150.0,
            86.0,
            "RESERVOIR",
            &[
                format!("{:.1} L", g(id, "fluid") * 1000.0),
                format!("{} air  {:.0} C", psi(g(id, "reservoir_pressure")), g(id, "temperature") - 273.15),
                format!("filter {:.0} psi", g(id, "filter_dp") / PA_PER_PSI),
                format!("relief {:.1}  leak {:.1} L/min", g(id, "relief_flow") * 60000.0, g(id, "leak_flow") * 60000.0),
            ],
            res_state,
        );

        // Manifold: a vertical pressure line the pumps feed from the left and the users draw from on the right.
        let mx = 440.0;
        let my0 = top - 16.0;
        let my1 = y0 + bh - 14.0;
        pen.line(&[(mx, my0), (mx, my1)], live);
        pen.line(&[(162.0, top + 43.0), (180.0, top + 43.0), (180.0, my1 - 4.0)], if g(id, "fluid") > 0.0 { State::Live } else { State::Off });
        pen.text(186.0, my1 - 10.0, "suction", 9.0, style::GREY_TEXT, Align2::LEFT_CENTER);

        // Pumps, each behind the valve that serves it.
        let pumps: Vec<usize> = (0..c.parts.len()).filter(|&i| c.parts[i].domain == "hydraulic" && c.parts[i].kind == "pump" && c.parts[i].part("circuit") == Some(ci)).collect();
        let valves: Vec<usize> = (0..c.parts.len()).filter(|&i| c.parts[i].domain == "hydraulic" && c.parts[i].kind == "valve" && c.parts[i].part("circuit") == Some(ci)).collect();
        let mut py = top;
        for &pi in &pumps {
            let pid = c.parts[pi].id.clone();
            let running = g(&pid, "running") != 0.0;
            let st = if g(&pid, "failed") != 0.0 { State::Fail } else if g(&pid, "cavitating") != 0.0 || g(&pid, "overheat") != 0.0 { State::Warn } else if running { State::Live } else { State::Off };
            let fsov = valves.iter().copied().find(|&v| c.parts[v].parts("serves").contains(&pi));
            pen.line(&[(180.0, py + 13.0), (200.0, py + 13.0)], if running { State::Live } else { State::Off });
            if let Some(v) = fsov {
                let vid = c.parts[v].id.clone();
                let open = g(&vid, "open") != 0.0;
                let vs = if g(&vid, "failed") != 0.0 || g(&vid, "stuck") != 0.0 { State::Fail } else if open { State::Live } else { State::Warn };
                pen.part(v, 200.0, py, 46.0, 26.0, if open { "FSOV" } else { "SHUT" }, &[], vs);
            }
            pen.line(&[(246.0, py + 13.0), (262.0, py + 13.0)], if running { State::Live } else { State::Off });
            pen.part(pi, 262.0, py, 120.0, 26.0, &style::label(&pid), &[], st);
            pen.text(386.0, py + 13.0, &format!("{:.0}", g(&pid, "flow") * 60000.0), 9.5, style::GREY_TEXT, Align2::LEFT_CENTER);
            pen.line(&[(382.0, py + 13.0), (mx, py + 13.0)], if running && st != State::Fail { State::Live } else { State::Off });
            py += 30.0;
        }
        pen.text(386.0, top - 6.0, "L/min", 8.5, style::GREY_TEXT, Align2::LEFT_CENTER);

        // Accumulators and the valves that are not a pump's.
        let accs: Vec<usize> = (0..c.parts.len()).filter(|&i| c.parts[i].domain == "hydraulic" && c.parts[i].kind == "accumulator" && c.parts[i].part("circuit") == Some(ci)).collect();
        let mut ay = top.max(py + 6.0);
        for &a in &accs {
            let aid = c.parts[a].id.clone();
            let p = g(&aid, "pressure");
            let st = if g(&aid, "failed") != 0.0 { State::Fail } else if p > 1000.0 * PA_PER_PSI { State::Live } else { State::Off };
            pen.part(a, 262.0, ay, 120.0, 34.0, &style::label(&aid), &[psi(p)], st);
            pen.line(&[(382.0, ay + 17.0), (mx, ay + 17.0)], st);
            ay += 38.0;
        }
        let others: Vec<usize> = valves.iter().copied().filter(|&v| !pumps.iter().any(|&p| c.parts[v].parts("serves").contains(&p))).collect();

        // Users, in columns to the right of the manifold.
        let u = &bands[n];
        let col = |k: usize| 470.0 + 260.0 * k as f32;
        let header = |pen: &Pen, k: usize, t: &str| pen.text(col(k), top - 4.0, t, 10.5, style::GREY_TEXT, Align2::LEFT_CENTER);
        header(&pen, 0, "FLIGHT CONTROLS");
        let mut fy = top + 6.0;
        for &si in &u.surfaces {
            let sp = &c.parts[si];
            let sid = sp.id.clone();
            let failed = g(&sid, "failed") != 0.0;
            let acts: Vec<String> = sp
                .parts("actuators")
                .iter()
                .map(|&a| {
                    let ap = &c.parts[a];
                    let tag = match ap.part("circuit").map(|k| c.parts[k].id.as_str()) {
                        Some(cid) => cid.chars().next().unwrap_or('?').to_ascii_uppercase(),
                        None => 'E',
                    };
                    let mark = if g(&ap.id, "failed") != 0.0 { "x" } else if g(&ap.id, "active") != 0.0 { "*" } else { "." };
                    format!("{tag}{mark}")
                })
                .collect();
            let st = if failed { State::Fail } else if g(&sid, "active_actuators") > 0.0 { State::Live } else { State::Off };
            pen.part(si, col(0), fy, 120.0, 20.0, &style::label(&sid), &[], st);
            pen.text(col(0) + 126.0, fy + 10.0, &format!("{:+6.1}  {}", g(&sid, "angle").to_degrees(), acts.join(" ")), 9.5, style::text_of(st), Align2::LEFT_CENTER);
            pen.line(&[(mx, fy + 10.0), (col(0), fy + 10.0)], if st == State::Off { State::Off } else { live });
            fy += 22.0;
        }

        header(&pen, 1, "MOTORS, GEAR, STEERING");
        let mut gy = top + 6.0;
        for &mi in &u.motors {
            let id = c.parts[mi].id.clone();
            let value = if c.parts[mi].kind == "stabilizer" { format!("{:+.2} deg", g(&id, "angle").to_degrees()) } else { format!("{:.0} %", g(&id, "position") * 100.0) };
            let st = if g(&id, "failed") != 0.0 { State::Fail } else if g(&id, "hydraulic_motors") > 0.0 { State::Live } else { State::Off };
            pen.part(mi, col(1), gy, 120.0, 20.0, &style::label(&id), &[], st);
            pen.text(col(1) + 126.0, gy + 10.0, &value, 9.5, style::text_of(st), Align2::LEFT_CENTER);
            gy += 22.0;
        }
        for &li in &u.legs {
            let id = c.parts[li].id.clone();
            let state = if g(&id, "down_locked") != 0.0 { "DN LK" } else if g(&id, "up_locked") != 0.0 { "UP LK" } else { "TRANSIT" };
            let st = if g(&id, "failed") != 0.0 || g(&id, "disagree") != 0.0 { State::Fail } else if g(&id, "in_transit") != 0.0 { State::Warn } else { State::Live };
            pen.part(li, col(1), gy, 120.0, 20.0, &style::label(&id), &[], st);
            pen.text(col(1) + 126.0, gy + 10.0, &format!("{state} {}", if g(&id, "doors_closed") != 0.0 { "doors shut" } else { "doors open" }), 9.5, style::text_of(st), Align2::LEFT_CENTER);
            gy += 22.0;
        }
        for &ti in &u.steering {
            let id = c.parts[ti].id.clone();
            let st = if g(&id, "failed") != 0.0 { State::Fail } else if g(&id, "towing") != 0.0 { State::Warn } else { State::Live };
            pen.part(ti, col(1), gy, 120.0, 20.0, &style::label(&id), &[], st);
            pen.text(col(1) + 126.0, gy + 10.0, &format!("{:+.1} deg", g(&id, "angle").to_degrees()), 9.5, style::text_of(st), Align2::LEFT_CENTER);
            gy += 22.0;
        }
        if gy > top + 6.0 {
            pen.line(&[(mx, top - 16.0), (col(1) - 8.0, top - 16.0), (col(1) - 8.0, gy - 12.0)], live);
            let mut yy = top + 16.0;
            while yy < gy {
                pen.line(&[(col(1) - 8.0, yy), (col(1), yy)], live);
                yy += 22.0;
            }
        }

        header(&pen, 2, "BRAKES, VALVES, LINES");
        let mut by = top + 6.0;
        for (k, &wi) in u.brakes.iter().enumerate() {
            let id = c.parts[wi].id.clone();
            let p = g(&id, "pressure");
            let st = if g(&id, "failed") != 0.0 || g(&id, "burst") != 0.0 || g(&id, "fire") != 0.0 { State::Fail } else if g(&id, "hot") != 0.0 { State::Warn } else if p > 100.0 * PA_PER_PSI { State::Live } else { State::Off };
            let (cx, cy) = (col(2) + (k % 4) as f32 * 62.0, by + (k / 4) as f32 * 30.0);
            pen.part(wi, cx, cy, 58.0, 26.0, &style::label(&id).replace("WHEEL ", "W"), &[], st);
            pen.text(cx + 29.0, cy + 21.0, &format!("{:.0}", p / PA_PER_PSI), 8.0, style::GREY_TEXT, Align2::CENTER_CENTER);
        }
        by += ((u.brakes.len() + 3) / 4) as f32 * 30.0 + 6.0;
        for &v in &others {
            let vid = c.parts[v].id.clone();
            let open = g(&vid, "open") != 0.0;
            let st = if g(&vid, "failed") != 0.0 || g(&vid, "stuck") != 0.0 { State::Fail } else if open { State::Live } else { State::Warn };
            pen.part(v, col(2), by, 160.0, 20.0, &style::label(&vid), &[], st);
            pen.text(col(2) + 166.0, by + 10.0, if open { "open" } else { "shut" }, 9.5, style::text_of(st), Align2::LEFT_CENTER);
            by += 22.0;
        }
        for &li in &u.lines {
            let id = c.parts[li].id.clone();
            let st = if g(&id, "failed") != 0.0 || g(&id, "leak_flow") > 0.0 { State::Fail } else if g(&id, "powered") != 0.0 { State::Live } else { State::Off };
            pen.part(li, col(2), by, 160.0, 20.0, &style::label(&id), &[], st);
            pen.text(col(2) + 166.0, by + 10.0, &format!("{:.1} L/min", g(&id, "flow") * 60000.0), 9.5, style::text_of(st), Align2::LEFT_CENTER);
            by += 22.0;
        }
        if by > top + 6.0 {
            pen.line(&[(col(1) - 8.0, top - 16.0), (col(2) - 8.0, top - 16.0), (col(2) - 8.0, by - 12.0)], live);
        }
        pen.text(col(3) - 20.0, my1, "* active   . damping   x failed   G Y green yellow   E electrical", 9.0, style::GREY_TEXT, Align2::RIGHT_CENTER);
        y0 += bh + 12.0;
    }
    pen.hits
}
