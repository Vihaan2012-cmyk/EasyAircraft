//! Side view of a gas path, drawn from its stage list.

use easyacft_parts::engine::{GasPath, Params};
use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke};

pub struct Stage {
    pub name: &'static str,
    pub kind: char,
    pub spool: usize,
}

/// The stages a parameter set has, front to back: c compressor, b burner, t turbine.
pub fn stages(p: &Params) -> Vec<Stage> {
    let mut s = Vec::new();
    let lp_comp = if p.spools == 1 { 2 } else { 0 };
    if p.pr[0] > 1.0 {
        s.push(Stage { name: "Fan", kind: 'c', spool: lp_comp });
    }
    if p.pr[1] > 1.0 {
        s.push(Stage { name: "LPC", kind: 'c', spool: lp_comp });
    }
    if p.pr[2] > 1.0 {
        s.push(Stage { name: "IPC", kind: 'c', spool: if p.spools == 3 { 1 } else { lp_comp } });
    }
    if p.pr[3] > 1.0 {
        s.push(Stage { name: "HPC", kind: 'c', spool: 2 });
    }
    s.push(Stage { name: "Burner", kind: 'b', spool: 9 });
    s.push(Stage { name: "HPT", kind: 't', spool: 2 });
    if p.spools == 3 {
        s.push(Stage { name: "IPT", kind: 't', spool: 1 });
    }
    if p.spools >= 2 || p.design_power > 0.0 {
        s.push(Stage { name: if p.design_power > 0.0 { "Power turbine" } else { "LPT" }, kind: 't', spool: 0 });
    }
    s
}

pub fn draw(painter: &Painter, rect: Rect, p: &Params, get: &dyn Fn(&str) -> Option<f64>, ink: Color32, faint: Color32, fill: Color32) {
    let st = stages(p);
    let running = get("running").is_some_and(|v| v != 0.0);
    let line = Stroke::new(if running { 2.2f32 } else { 1.0 }, if running { ink } else { faint });
    let thin = Stroke::new(1.0f32, faint);
    let font = FontId::proportional(12.0);
    let small = FontId::proportional(10.5);
    let axis = rect.top() + rect.height() * 0.58;
    let left = rect.left() + 40.0;
    let width = rect.width() - 80.0;
    let unit = width / (st.len() as f32 + 2.0);
    let core = |k: f32| 70.0 - 30.0 * k;
    let bypass = p.bpr > 0.0 && p.pr[0] > 1.0;
    painter.line_segment([Pos2::new(left - 20.0, axis), Pos2::new(left + width + 20.0, axis)], Stroke::new(1.0f32, faint));
    let ncomp = st.iter().filter(|s| s.kind == 'c').count().max(1) as f32;
    let mut k_comp = 0.0;
    let mut k_turb = 0.0;
    let nturb = st.iter().filter(|s| s.kind == 't').count().max(1) as f32;
    let mut centres = Vec::new();
    for (i, s) in st.iter().enumerate() {
        let x0 = left + unit * (i as f32 + 1.0);
        let x1 = x0 + unit * 0.85;
        let (r0, r1) = match s.kind {
            'c' => {
                let a = core(k_comp / ncomp);
                k_comp += 1.0;
                (a, core(k_comp / ncomp))
            }
            'b' => (core(1.0), core(1.0)),
            _ => {
                let a = core(1.0 - k_turb / nturb);
                k_turb += 1.0;
                (a, core(1.0 - k_turb / nturb))
            }
        };
        let (r0, r1) = if s.name == "Fan" { (120.0, 120.0) } else { (r0, r1) };
        let pts = vec![Pos2::new(x0, axis), Pos2::new(x0, axis - r0), Pos2::new(x1, axis - r1), Pos2::new(x1, axis), Pos2::new(x0, axis)];
        painter.add(Shape::convex_polygon(pts[..4].to_vec(), fill, Stroke::NONE));
        painter.add(Shape::line(pts, line));
        if s.kind == 'b' {
            painter.circle_stroke(Pos2::new((x0 + x1) / 2.0, axis - r0 * 0.55), 8.0, line);
        } else {
            let n = 4;
            for b in 1..n {
                let x = x0 + (x1 - x0) * b as f32 / n as f32;
                let r = r0 + (r1 - r0) * b as f32 / n as f32;
                painter.line_segment([Pos2::new(x, axis - 4.0), Pos2::new(x, axis - r + 4.0)], thin);
            }
        }
        painter.text(Pos2::new((x0 + x1) / 2.0, axis - r0.max(r1) - 8.0), Align2::CENTER_BOTTOM, s.name, font.clone(), ink);
        centres.push((x0, x1, s));
    }
    if bypass {
        let (fx0, fx1, _) = centres[0];
        let end = centres.last().map_or(fx1, |c| c.1);
        let top = axis - 120.0;
        painter.line_segment([Pos2::new(fx0, top), Pos2::new(end - unit * 0.5, top)], line);
        painter.line_segment([Pos2::new(fx1, axis - 76.0), Pos2::new(end - unit * 1.2, axis - 76.0)], thin);
        painter.text(Pos2::new((fx1 + end) / 2.0, axis - 98.0), Align2::CENTER_CENTER, format!("bypass {:.1}:1", p.bpr), small.clone(), faint);
    }
    let names = ["LP", "IP", "HP"];
    let outs = ["n1", "n2", "n3"];
    let mut depth = 18.0;
    for spool in [2usize, 1, 0] {
        let xs: Vec<f32> = centres.iter().filter(|c| c.2.spool == spool).flat_map(|c| [c.0, c.1]).collect();
        if xs.len() < 2 {
            continue;
        }
        let (a, b) = (xs.iter().copied().fold(f32::MAX, f32::min), xs.iter().copied().fold(f32::MIN, f32::max));
        let y = axis + depth;
        painter.line_segment([Pos2::new(a + 10.0, y), Pos2::new(b - 10.0, y)], line);
        let out = match (p.spools, spool) {
            (1, _) => outs[0],
            (2, 2) => outs[1],
            (_, k) => outs[k],
        };
        let pct = get(out).map(|v| format!("{v:.1} %")).unwrap_or_default();
        painter.text(Pos2::new(b - 10.0, y + 3.0), Align2::RIGHT_TOP, format!("{} spool {:.0} rpm  {pct}", names[spool], p.rpm[spool]), small.clone(), faint);
        depth += 26.0;
    }
    let stat = |label: &str, text: String, x: f32| {
        painter.text(Pos2::new(x, axis + 100.0), Align2::CENTER_TOP, label, small.clone(), faint);
        painter.text(Pos2::new(x, axis + 114.0), Align2::CENTER_TOP, text, font.clone(), ink);
    };
    let c = |n: &str| get(n).unwrap_or(0.0);
    let find = |n: &str| centres.iter().find(|x| x.2.name == n).map(|x| (x.0 + x.1) / 2.0);
    if let Some(x) = find("HPC").or_else(|| find("IPC")) {
        stat("P3 / T3", format!("{:.0} psi  {:.0} °C", c("p3") / 6894.757, c("t3") - 273.15), x);
    }
    if let Some(x) = find("Burner") {
        stat("TET / fuel", format!("{:.0} °C  {:.3} kg/s", c("tet") - 273.15, c("fuel_flow")), x);
    }
    if let Some(x) = centres.last().map(|x| x.1 + unit * 0.6) {
        stat("EGT / thrust", format!("{:.0} °C  {:.1} kN", c("egt") - 273.15, c("thrust") / 1e3), x);
    }
    if let Some((x, _, _)) = centres.first() {
        stat("airflow", format!("{:.1} kg/s", c("airflow")), *x);
    }
}

/// The design point table: station temperatures and turbine pressure ratios.
pub fn design_rows(g: &GasPath) -> Vec<(String, String)> {
    let p = &g.p;
    let d = &g.d;
    let mut rows = Vec::new();
    for (k, n) in ["Fan", "LPC", "IPC", "HPC"].iter().enumerate() {
        if p.pr[k] > 1.0 {
            rows.push((format!("{n} pressure ratio, inlet"), format!("{:.2}, {:.0} K", p.pr[k], d.t_in[k])));
        }
    }
    let opr: f64 = p.pr.iter().product();
    rows.push(("Overall pressure ratio".into(), format!("{opr:.1}")));
    rows.push(("Turbine entry".into(), format!("{:.0} K, {:.0} kPa", d.t4, d.p4 / 1e3)));
    rows.push(("Turbine pressure ratios HP, IP, LP".into(), format!("{:.2}, {:.2}, {:.2}", d.pr_t[2], d.pr_t[1], d.pr_t[0])));
    rows.push(("Fuel at design".into(), format!("{:.3} kg/s", d.wf)));
    rows.push(("Thrust at design".into(), format!("{:.1} kN", d.thrust / 1e3)));
    if d.t4 >= 2399.0 || d.t4 <= 901.0 {
        rows.push(("Design point".into(), "does not close: check pressure ratios, efficiencies and airflow".into()));
    }
    rows
}

/// Applies edited settings to engine parameters so the design point solves as you type.
pub fn with_edits(mut p: Params, edits: &std::collections::HashMap<String, String>) -> Params {
    let num = |k: &str| -> Option<f64> {
        let t = edits.get(k)?.trim().trim_matches('"');
        t.parse::<f64>().ok().or_else(|| easyacft_core::parse_quantity(t).ok().map(|q| q.si))
    };
    let set = |v: &mut f64, k: &str| {
        if let Some(x) = num(k) {
            *v = x;
        }
    };
    set(&mut p.design_thrust, "design_thrust");
    set(&mut p.design_power, "design_power");
    set(&mut p.airflow, "airflow");
    set(&mut p.bpr, "bypass_ratio");
    for (i, k) in ["fan_pr", "lpc_pr", "ipc_pr", "hpc_pr"].iter().enumerate() {
        set(&mut p.pr[i], k);
    }
    for (i, k) in ["fan_efficiency", "lpc_efficiency", "ipc_efficiency", "hpc_efficiency"].iter().enumerate() {
        set(&mut p.eta_c[i], k);
    }
    for (i, k) in ["hpt_efficiency", "ipt_efficiency", "lpt_efficiency"].iter().enumerate() {
        set(&mut p.eta_t[i], k);
    }
    set(&mut p.eta_b, "burner_efficiency");
    set(&mut p.loss_b, "burner_loss");
    set(&mut p.cooling, "cooling");
    if let Some(s) = num("spools") {
        p.spools = s.clamp(1.0, 3.0) as usize;
    }
    p
}
