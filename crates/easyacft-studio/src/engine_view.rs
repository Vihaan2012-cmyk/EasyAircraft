//! Engine design-point table and live edits of engine settings.

use easyacft_parts::engine::{GasPath, Params};

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
