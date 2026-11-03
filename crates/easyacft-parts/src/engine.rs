//! Engines: a configurable gas path (turbojet, turbofan, geared fan, three-spool, turboprop),
//! a piston engine, propellers and the engine fit.

use crate::util::{Packer, Slots};
use crate::{expr_of, Domain, Expr};
use easyacft_core::Dim;
use easyacft_lang::expr::Table;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const F: Type = Type::Num(Dim::FORCE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const P: Type = Type::Num(Dim::PRESSURE);
const W: Type = Type::Num(Dim::POWER);
const MF: Type = Type::Num(Dim::MASS_FLOW);
const RPM: Type = Type::Num(Dim::FREQUENCY);
const AIR: &[&str] = &["environment.air"];
const WHEN: Kind = Kind::Expr(Type::Bool);
const SPECIFIC_ENERGY: Dim = Dim([0, 2, -2, 0, 0]);

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "engine",
        kind: "gas_turbine",
        help: "A gas turbine built from its design point: turbojet, turbofan, geared fan, three-spool or turboprop.",
        fields: &[
            req("air", Kind::Ref(AIR), "outside air"),
            req("fuel_flow", Kind::Expr(MF), "fuel reaching the burner, for example \"eng1_feed.flow\""),
            opt("design_thrust", Kind::Quantity(Dim::FORCE), "0 N", "static sea-level thrust it is built to; 0 for a turboprop"),
            opt("design_power", Kind::Quantity(Dim::POWER), "0 W", "shaft power at design, for a turboprop or turboshaft"),
            req("airflow", Kind::Quantity(Dim::MASS_FLOW), "total airflow at design"),
            opt("bypass_ratio", Kind::Number, "0", "0 for a turbojet or turboprop"),
            opt("spools", Kind::Number, "2", "1, 2 or 3"),
            opt("fan_pr", Kind::Number, "1.0", "fan pressure ratio; 1 for none"),
            opt("lpc_pr", Kind::Number, "1.0", "booster pressure ratio on the low-pressure spool"),
            opt("ipc_pr", Kind::Number, "1.0", "intermediate compressor pressure ratio, three spools"),
            req("hpc_pr", Kind::Number, "high-pressure compressor pressure ratio"),
            opt("fan_efficiency", Kind::Number, "0.90", "isentropic"),
            opt("lpc_efficiency", Kind::Number, "0.89", "isentropic"),
            opt("ipc_efficiency", Kind::Number, "0.89", "isentropic"),
            opt("hpc_efficiency", Kind::Number, "0.86", "isentropic"),
            opt("hpt_efficiency", Kind::Number, "0.90", "isentropic"),
            opt("ipt_efficiency", Kind::Number, "0.91", "isentropic"),
            opt("lpt_efficiency", Kind::Number, "0.92", "isentropic"),
            opt("burner_efficiency", Kind::Number, "0.995", "share of the fuel's heat released"),
            opt("burner_loss", Kind::Number, "0.05", "total-pressure loss share"),
            opt("fuel_heat", Kind::Quantity(SPECIFIC_ENERGY), "43.1 MJ/kg", "lower heating value"),
            opt("cooling", Kind::Number, "0", "share of core air cooling the turbine, rejoining after it"),
            opt("lp_rpm", Kind::Quantity(Dim::FREQUENCY), "3000 rpm", "low-pressure spool speed at design"),
            opt("ip_rpm", Kind::Quantity(Dim::FREQUENCY), "8000 rpm", "intermediate spool speed at design"),
            opt("hp_rpm", Kind::Quantity(Dim::FREQUENCY), "12000 rpm", "high-pressure spool speed at design"),
            opt("lp_inertia", Kind::Number, "1000", "kg m2"),
            opt("ip_inertia", Kind::Number, "60", "kg m2"),
            opt("hp_inertia", Kind::Number, "30", "kg m2"),
            opt("fan_gear", Kind::Number, "1", "low-pressure turbine to fan speed ratio"),
            opt("output_rpm", Kind::Expr(RPM), "0 rpm", "power turbine speed when it drives a propeller"),
            opt("starter_torque", Kind::Expr(Type::Num(Dim::TORQUE)), "0 N*m", "on the high-pressure spool"),
            opt("offtake", Kind::Expr(W), "0 W", "gearbox loads: generators, pumps"),
            opt("bleed", Kind::Expr(MF), "0 kg/s", "customer bleed taken from the high-pressure compressor"),
            opt("reverser", Kind::Expr(N), "0", "reverser deployed 0 to 1"),
            opt("reverse_efficiency", Kind::Number, "0.35", "share of fan thrust a deployed reverser turns back"),
            opt("hpc_map", Kind::Text, "", "CSV: corrected speed, flow share, pressure-ratio share, efficiency share"),
            opt("fan_map", Kind::Text, "", "CSV, as hpc_map"),
        ],
        outputs: &[
            ("thrust", F),
            ("n1", N),
            ("n2", N),
            ("n3", N),
            ("egt", T),
            ("tet", T),
            ("p3", P),
            ("t3", T),
            ("p_ip", P),
            ("fuel_flow", MF),
            ("airflow", MF),
            ("shaft_power", W),
            ("running", B),
            ("failed", B),
        ],
    },
    PartSpec {
        domain: "output",
        kind: "force",
        help: "A force the plugin applies to X-Plane's aircraft, such as an engine's thrust.",
        fields: &[
            req("force", Kind::Expr(F), "for example \"eng1.thrust\""),
            req("x", Kind::Quantity(Dim::LENGTH), "right of the centre of gravity"),
            req("y", Kind::Quantity(Dim::LENGTH), "above the centre of gravity"),
            req("z", Kind::Quantity(Dim::LENGTH), "aft of the centre of gravity"),
            opt("pitch", Kind::Quantity(Dim::NONE), "0 deg", "nose-up tilt of the force line"),
            opt("yaw", Kind::Quantity(Dim::NONE), "0 deg", "rightward tilt of the force line"),
            opt("scale", Kind::Number, "1", "multiplier"),
            opt("replaces_engines", Kind::Bool, "false", "take over X-Plane's own engine forces"),
        ],
        outputs: &[("force", F)],
    },
    PartSpec {
        domain: "engine",
        kind: "piston",
        help: "A piston engine.",
        fields: &[
            req("air", Kind::Ref(AIR), "outside air"),
            req("rated_power", Kind::Quantity(Dim::POWER), "for example \"180 hp\""),
            opt("rated_rpm", Kind::Quantity(Dim::FREQUENCY), "2700 rpm", "speed at rated power"),
            req("rpm", Kind::Expr(RPM), "crankshaft speed, from its propeller: \"prop.rpm\""),
            opt("throttle", Kind::Expr(N), "0", "0 to 1"),
            opt("mixture", Kind::Expr(N), "1", "0 cut-off to 1 full rich"),
            opt("magnetos", Kind::Expr(N), "2", "magnetos on: 0, 1 or 2"),
            opt("fuel_available", WHEN, "true", "when fuel reaches it"),
            opt("turbocharged", Kind::Bool, "false", "holds sea-level manifold pressure with altitude"),
            opt("bsfc", Kind::Number, "0.45", "best-power fuel use, lb per hp per hour"),
        ],
        outputs: &[("power", W), ("manifold_pressure", P), ("fuel_flow", MF), ("egt", T), ("cht", T), ("running", B), ("failed", B)],
    },
    PartSpec {
        domain: "engine",
        kind: "propeller",
        help: "A fixed, constant-speed or reversing propeller with its speed.",
        fields: &[
            req("air", Kind::Ref(AIR), "outside air"),
            req("power", Kind::Expr(W), "shaft power driving it: \"eng1.power\" or \"eng1.shaft_power\""),
            req("diameter", Kind::Quantity(Dim::LENGTH), "for example \"1.9 m\""),
            req("rated_rpm", Kind::Quantity(Dim::FREQUENCY), "speed at rated power"),
            req("rated_power", Kind::Quantity(Dim::POWER), "power it absorbs at rated speed and fine pitch"),
            opt("inertia", Kind::Number, "3", "kg m2 including the engine"),
            opt("efficiency", Kind::Number, "0.85", "peak propulsive efficiency"),
            opt("governor", Kind::Expr(RPM), "0 rpm", "speed the governor holds; 0 for fixed pitch"),
            opt("feather_when", WHEN, "false", "feathered"),
            opt("beta", Kind::Expr(N), "0", "reverse pitch 0 to 1"),
            opt("starter_torque", Kind::Expr(Type::Num(Dim::TORQUE)), "0 N*m", "starter on the shaft"),
            opt("friction", Kind::Quantity(Dim::POWER), "0 W", "engine friction at rated speed"),
        ],
        outputs: &[("rpm", RPM), ("thrust", F), ("pitch", N), ("feathered", B), ("failed", B)],
    },
];

const CP_AIR: f64 = 1005.0;
const CP_GAS: f64 = 1150.0;
const KA: f64 = 0.4 / 1.4;
const KG: f64 = 0.333 / 1.333;
const MECH: f64 = 0.99;
const FAR_MAX: f64 = 0.068;
const TAU: f64 = std::f64::consts::TAU;

fn h_air(t: f64) -> f64 {
    t * (954.8 + 0.5 * 0.1742 * t)
}

/// Engine parameters, everything a developer can set.
#[derive(Clone, Debug, Default)]
pub struct Params {
    pub design_thrust: f64,
    pub design_power: f64,
    pub airflow: f64,
    pub bpr: f64,
    pub spools: usize,
    pub pr: [f64; 4],
    pub eta_c: [f64; 4],
    pub eta_t: [f64; 3],
    pub eta_b: f64,
    pub loss_b: f64,
    pub lhv: f64,
    pub cooling: f64,
    pub rpm: [f64; 3],
    pub inertia: [f64; 3],
    pub fan_gear: f64,
    pub rev_eff: f64,
    pub hpc_map: Option<Table>,
    pub fan_map: Option<Table>,
}

impl Params {
    pub fn from_part(p: &CheckedPart, base: &std::path::Path) -> Self {
        let map = |k: &str| -> Option<Table> {
            let t = p.text(k);
            (!t.is_empty()).then(|| std::fs::read_to_string(base.join(t)).ok().and_then(|s| Table::parse(&s).ok())).flatten()
        };
        Params {
            design_thrust: p.num("design_thrust"),
            design_power: p.num("design_power"),
            airflow: p.num("airflow").max(0.01),
            bpr: p.num("bypass_ratio").max(0.0),
            spools: p.num("spools").clamp(1.0, 3.0) as usize,
            pr: [p.num("fan_pr").max(1.0), p.num("lpc_pr").max(1.0), p.num("ipc_pr").max(1.0), p.num("hpc_pr").max(1.0)],
            eta_c: [p.num("fan_efficiency"), p.num("lpc_efficiency"), p.num("ipc_efficiency"), p.num("hpc_efficiency")],
            eta_t: [p.num("hpt_efficiency"), p.num("ipt_efficiency"), p.num("lpt_efficiency")],
            eta_b: p.num("burner_efficiency"),
            loss_b: p.num("burner_loss"),
            lhv: p.num("fuel_heat"),
            cooling: p.num("cooling").clamp(0.0, 0.4),
            rpm: [p.num("lp_rpm") * 60.0, p.num("ip_rpm") * 60.0, p.num("hp_rpm") * 60.0],
            inertia: [p.num("lp_inertia").max(0.01), p.num("ip_inertia").max(0.01), p.num("hp_inertia").max(0.01)],
            fan_gear: p.num("fan_gear").max(0.1),
            rev_eff: p.num("reverse_efficiency"),
            hpc_map: map("hpc_map"),
            fan_map: map("fan_map"),
        }
    }
}

/// Inputs for one gas-path evaluation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flight {
    pub p0: f64,
    pub t0: f64,
    pub v0: f64,
    pub mach: f64,
    pub fuel: f64,
    pub bleed: f64,
    pub output_rpm: f64,
}

impl Flight {
    pub fn static_isa() -> Self {
        Flight { p0: 101_325.0, t0: 288.15, ..Default::default() }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Gas {
    pub thrust: f64,
    pub w: f64,
    pub wf: f64,
    pub t3: f64,
    pub p3: f64,
    pub p_ip: f64,
    pub t4: f64,
    pub egt: f64,
    pub shaft: f64,
    /// Turbine minus compressor power per spool (LP, IP, HP), watts.
    pub surplus: [f64; 3],
}

/// What the design point fixes.
#[derive(Clone, Debug, Default)]
pub struct Design {
    pub t_in: [f64; 4],
    pub t4: f64,
    pub p4: f64,
    pub pr_t: [f64; 3],
    pub wf: f64,
    pub thrust: f64,
}

fn map_at(t: &Option<Table>, nc: f64, pr_d: f64, eta_d: f64, w_d: f64) -> (f64, f64, f64) {
    match t {
        Some(t) if !t.cols.is_empty() || !t.values.is_empty() => {
            let col = |c: usize| {
                let ys: Vec<f64> = t.values.iter().map(|v| v.get(c).copied().unwrap_or(1.0)).collect();
                let k = t.xs.iter().position(|&x| x >= nc).unwrap_or(t.xs.len() - 1).max(1);
                let (x0, x1) = (t.xs[k - 1], t.xs[k]);
                let s = ((nc - x0) / (x1 - x0).max(1e-9)).clamp(0.0, 1.0);
                ys[k - 1] + s * (ys[k] - ys[k - 1])
            };
            (w_d * col(0), 1.0 + (pr_d - 1.0) * col(1), eta_d * col(2))
        }
        _ => {
            let eta = (eta_d * (1.0 - 0.3 * (nc - 1.0).powi(2))).clamp(0.4, eta_d);
            (w_d * nc.max(0.0), 1.0 + (pr_d - 1.0) * nc.max(0.0).powi(2), eta)
        }
    }
}

pub struct GasPath {
    pub p: Params,
    pub d: Design,
}

impl GasPath {
    /// Builds the engine, solving turbine entry temperature and turbine pressure ratios at sea-level static.
    pub fn new(p: Params) -> Self {
        let mut g = GasPath { p, d: Design::default() };
        let turboprop = g.p.design_power > 0.0;
        let (mut lo, mut hi) = (900.0, 2400.0);
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            let ok = match g.design_at(mid) {
                Some(d) if turboprop => d.thrust >= 0.04 * g.p.design_power / 100.0,
                Some(d) => d.thrust >= g.p.design_thrust,
                None => false,
            };
            if ok {
                hi = mid
            } else {
                lo = mid
            }
        }
        g.d = g.design_at(hi).unwrap_or_default();
        g
    }

    fn design_at(&self, t4: f64) -> Option<Design> {
        let p = &self.p;
        let (mut t, mut pr) = (288.15, 101_325.0 * 0.99);
        let w = p.airflow;
        let wc = w / (1.0 + p.bpr);
        let wb = w - wc;
        let comp = |t: f64, r: f64, eta: f64| t * (1.0 + (r.powf(KA) - 1.0) / eta.max(0.1));
        let mut t_in = [t; 4];
        let t13 = comp(t, p.pr[0], p.eta_c[0]);
        let p13 = pr * p.pr[0];
        let fan_power = w * CP_AIR * (t13 - t);
        t = t13;
        pr = p13;
        t_in[1] = t;
        let t_l = comp(t, p.pr[1], p.eta_c[1]);
        let lpc_power = wc * CP_AIR * (t_l - t);
        t = t_l;
        pr *= p.pr[1];
        t_in[2] = t;
        let t_i = comp(t, p.pr[2], p.eta_c[2]);
        let ipc_power = wc * CP_AIR * (t_i - t);
        t = t_i;
        pr *= p.pr[2];
        t_in[3] = t;
        let t3 = comp(t, p.pr[3], p.eta_c[3]);
        let hpc_power = wc * CP_AIR * (t3 - t);
        let p3 = pr * p.pr[3];
        let cool = wc * p.cooling;
        let wburn = wc - cool;
        let dh = h_air(t4) - h_air(t3);
        if dh <= 0.0 {
            return None;
        }
        let wf = wburn * dh / (p.lhv * p.eta_b - dh);
        let p4 = p3 * (1.0 - p.loss_b);
        let mut wg = wburn + wf;
        let mut tg = t4;
        let mut pg = p4;
        let mut pr_t = [1.0; 3];
        let expand = |power: f64, eta: f64, wg: f64, tg: &mut f64, pg: &mut f64| -> Option<f64> {
            let dt = power / MECH / (wg * CP_GAS);
            let tis = *tg - dt / eta.max(0.1);
            if tis <= 0.0 {
                return None;
            }
            let r = (*tg / tis).powf(1.0 / KG);
            *tg -= dt;
            *pg /= r;
            Some(r)
        };
        pr_t[2] = expand(hpc_power + if p.spools == 1 { fan_power + lpc_power } else { 0.0 }, p.eta_t[0], wg, &mut tg, &mut pg)?;
        tg = (wg * CP_GAS * tg + cool * CP_AIR * t3) / (wg * CP_GAS + cool * CP_AIR);
        wg += cool;
        if p.spools == 3 {
            pr_t[1] = expand(ipc_power, p.eta_t[1], wg, &mut tg, &mut pg)?;
        }
        let lp_power = if p.design_power > 0.0 { p.design_power } else if p.spools >= 2 { fan_power + lpc_power + if p.spools == 2 { ipc_power } else { 0.0 } } else { 0.0 };
        if lp_power > 0.0 {
            pr_t[0] = expand(lp_power, p.eta_t[2], wg, &mut tg, &mut pg)?;
        }
        if pg <= 101_325.0 * 1.001 {
            return None;
        }
        let v5 = (2.0 * CP_GAS * tg * (1.0 - (101_325.0 / pg).powf(KG))).max(0.0).sqrt();
        let v13 = (2.0 * CP_AIR * t13 * (1.0 - (101_325.0 / p13).powf(KA))).max(0.0).sqrt();
        Some(Design { t_in, t4, p4, pr_t, wf, thrust: wg * v5 * 0.98 + wb * v13 * 0.98 })
    }

    /// Steady gas path at spool speeds `n` (LP, IP, HP as shares of design).
    pub fn eval(&self, f: &Flight, n: [f64; 3]) -> Gas {
        let p = &self.p;
        let tt2 = f.t0 * (1.0 + 0.2 * f.mach * f.mach);
        let pt2 = f.p0 * (1.0 + 0.2 * f.mach * f.mach).powf(3.5) * 0.99;
        let (theta, delta) = (tt2 / 288.15, pt2 / 101_325.0);
        let spool_of = |stage: usize| match (stage, p.spools) {
            (3, _) => 2,
            (_, 1) => 2,
            (2, 3) => 1,
            (2, 2) => 0,
            _ => 0,
        };
        let flow = |stage: usize, map: &Option<Table>| {
            let nc = n[spool_of(stage)] / theta.sqrt();
            if map.is_some() { map_at(map, nc, 1.0, 1.0, 1.0).0 } else { nc.max(0.0) }
        };
        let core_stage = if p.pr[3] > 1.0 { 3 } else if p.pr[2] > 1.0 { 2 } else { 1 };
        let wc = p.airflow / (1.0 + p.bpr) * flow(core_stage, &p.hpc_map) * delta / theta.sqrt();
        let wb = p.airflow * p.bpr / (1.0 + p.bpr) * flow(0, &p.fan_map) * delta / theta.sqrt();
        let w = wc + wb;
        let (mut t, mut pr) = (tt2, pt2);
        let mut power = [0.0; 3];
        let mut t13 = tt2;
        let mut p13 = pt2;
        let mut p_ip = pt2;
        for (k, mass) in [(0usize, w), (1, wc), (2, wc), (3, wc)] {
            if p.pr[k] <= 1.0 {
                continue;
            }
            let nc = n[spool_of(k)] / (t / self.d.t_in[k].max(1.0)).sqrt();
            let mapped = match k {
                0 => map_at(&p.fan_map, nc, p.pr[k], p.eta_c[k], 1.0),
                3 => map_at(&p.hpc_map, nc, p.pr[k], p.eta_c[k], 1.0),
                _ => map_at(&None, nc, p.pr[k], p.eta_c[k], 1.0),
            };
            let r = mapped.1.max(1.0);
            let t_out = t * (1.0 + (r.powf(KA) - 1.0) / mapped.2.max(0.1));
            power[spool_of(k)] += mass * CP_AIR * (t_out - t);
            t = t_out;
            pr *= r;
            if k == 0 {
                t13 = t;
                p13 = pr;
            }
            if k < 3 {
                p_ip = pr;
            }
        }
        let (t3, p3) = (t, pr);
        let cool = wc * p.cooling;
        let wburn = (wc - cool - f.bleed).max(0.0);
        let wf = f.fuel.min(wburn * FAR_MAX).max(0.0);
        let t4 = if wburn + wf > 1e-6 { crate::engine::t_from_h(h_air(t3) + wf * p.lhv * p.eta_b / (wburn + wf)) } else { t3 };
        let p4 = p3 * (1.0 - p.loss_b);
        let total = (p4 / f.p0.max(1.0)).max(1.0).ln();
        let total_d = (self.d.p4 / 101_325.0).max(1.0001).ln();
        let share = (total / total_d).clamp(0.0, 1.2);
        let turbine_d: f64 = self.d.pr_t.iter().map(|r| r.max(1.0).ln()).sum();
        let reserve = (total_d - turbine_d).max(0.0) * share;
        let mut left = (total - reserve).max(0.0);
        let mut wg = wburn + wf;
        let (mut tg, mut pg) = (t4, p4);
        let mut turb = [0.0; 3];
        let mut stage = |idx: usize, eta: f64, wg: f64, tg: &mut f64, pg: &mut f64| -> f64 {
            let ln_d = self.d.pr_t[idx].max(1.0).ln();
            let ln = (ln_d * share.sqrt().min(1.2)).min(left);
            left -= ln;
            let r = ln.exp();
            if r <= 1.0 {
                return 0.0;
            }
            let dt = *tg * eta * (1.0 - r.powf(-KG));
            *tg -= dt;
            *pg /= r;
            wg * CP_GAS * dt * MECH
        };
        turb[2] = stage(2, p.eta_t[0], wg, &mut tg, &mut pg);
        tg = (wg * CP_GAS * tg + cool * CP_AIR * t3) / (wg * CP_GAS + cool * CP_AIR).max(1e-9);
        wg += cool;
        if p.spools == 3 {
            turb[1] = stage(1, p.eta_t[1], wg, &mut tg, &mut pg);
        }
        let egt = tg;
        let mut shaft = 0.0;
        if p.spools >= 2 || p.design_power > 0.0 {
            let lp = stage(0, p.eta_t[2], wg, &mut tg, &mut pg);
            if p.design_power > 0.0 {
                let speed = if f.output_rpm > 0.0 { (f.output_rpm / p.rpm[0].max(1.0)).clamp(0.0, 1.3) } else { 1.0 };
                shaft = lp * (1.0 - 0.6 * (speed - 1.0).powi(2)).max(0.0);
            } else {
                turb[0] = lp;
            }
        }
        if p.spools == 1 {
            turb[2] += turb[0];
            turb[0] = 0.0;
        }
        let v5 = if pg > f.p0 { (2.0 * CP_GAS * tg * (1.0 - (f.p0 / pg).powf(KG))).max(0.0).sqrt() } else { 0.0 };
        let v13 = if p13 > f.p0 { (2.0 * CP_AIR * t13 * (1.0 - (f.p0 / p13).powf(KA))).max(0.0).sqrt() } else { 0.0 };
        let thrust = wg * v5 * 0.98 + wb * v13 * 0.98 - w * f.v0;
        Gas { thrust, w, wf, t3, p3, p_ip, t4, egt, shaft, surplus: [turb[0] - power[0], turb[1] - power[1], turb[2] - power[2]] }
    }
}

pub fn t_from_h(h: f64) -> f64 {
    let (a, b) = (954.8, 0.5 * 0.1742);
    2.0 * h.max(0.0) / (a + (a * a + 4.0 * b * h.max(0.0)).sqrt())
}

/// A gas turbine with its spools turning.
pub struct Turbine {
    pub path: GasPath,
    pub n: [f64; 3],
    pub last: Gas,
}

impl Turbine {
    pub fn new(p: Params, start_running: bool) -> Self {
        let path = GasPath::new(p);
        let n = if start_running { [1.0; 3] } else { [0.0; 3] };
        Turbine { path, n, last: Gas::default() }
    }

    /// Advances the spools by `dt`; `starter` is torque on the HP spool, `offtake` watts from it.
    pub fn step(&mut self, f: &Flight, starter: f64, offtake: f64, dt: f64) -> Gas {
        let g = self.path.eval(f, self.n);
        let p = &self.path.p;
        let spools: &[usize] = match p.spools {
            1 => &[2],
            2 => &[0, 2],
            _ => &[0, 1, 2],
        };
        for &s in spools {
            if s == 0 && p.design_power > 0.0 {
                self.n[0] = (f.output_rpm / p.rpm[0].max(1.0)).max(0.0);
                continue;
            }
            let rpm_d = p.rpm[s].max(1.0);
            let omega = (self.n[s] * rpm_d * TAU / 60.0).max(5.0);
            let mut net = g.surplus[s] - 0.002 * self.path.d.wf * p.lhv * self.n[s].powi(2) / spools.len() as f64;
            if s == 2 {
                net += starter * omega - offtake;
            }
            let domega = net / (p.inertia[s] * omega);
            self.n[s] = (self.n[s] + domega * dt * 60.0 / (TAU * rpm_d)).clamp(0.0, 1.25);
        }
        self.last = g;
        g
    }

    /// Steady state at fuel flow `fuel`, run in time to settle.
    pub fn settle(&mut self, f: &Flight, seconds: f64) -> Gas {
        let mut g = Gas::default();
        for _ in 0..(seconds / 0.01) as usize {
            g = self.step(f, 0.0, 0.0, 0.01);
        }
        g
    }
}

struct GtPart {
    engine: Turbine,
    air: [usize; 3],
    fuel: Option<Expr>,
    starter: Option<Expr>,
    offtake: Option<Expr>,
    bleed: Option<Expr>,
    reverser: Option<Expr>,
    output_rpm: Option<Expr>,
    s: Slots,
}

struct Piston {
    air: [usize; 2],
    rated: f64,
    rated_rpm: f64,
    rpm: Option<Expr>,
    throttle: Option<Expr>,
    mixture: Option<Expr>,
    mags: Option<Expr>,
    fuel_ok: Option<Expr>,
    turbo: bool,
    bsfc: f64,
    egt: f64,
    cht: f64,
    s: Slots,
}

struct Prop {
    air: [usize; 3],
    power: Option<Expr>,
    diameter: f64,
    rated_rpm: f64,
    rated_power: f64,
    inertia: f64,
    eta: f64,
    governor: Option<Expr>,
    feather: Option<Expr>,
    beta: Option<Expr>,
    starter: Option<Expr>,
    friction: f64,
    rpm: f64,
    pitch: f64,
    s: Slots,
}

pub struct Engines {
    turbines: Vec<GtPart>,
    pistons: Vec<Piston>,
    props: Vec<Prop>,
    forces: Vec<(Option<Expr>, usize)>,
}

fn air_slots(c: &Checked, p: &CheckedPart, names: &[&str]) -> Option<Vec<usize>> {
    let a = &c.parts[p.part("air")?];
    names.iter().map(|n| c.slots.index.get(&format!("{}.{n}", a.id)).copied()).collect()
}

impl Engines {
    pub fn new(c: &Checked, base: &std::path::Path) -> Option<Self> {
        let mut e = Engines { turbines: vec![], pistons: vec![], props: vec![], forces: vec![] };
        for p in c.parts.iter().filter(|p| p.domain == "output" && p.kind == "force") {
            e.forces.push((expr_of(p, "force"), crate::catalog::out(p, "force")));
        }
        for p in c.parts.iter().filter(|p| p.domain == "engine") {
            match p.kind {
                "gas_turbine" => {
                    let Some(air) = air_slots(c, p, &["pressure", "temperature", "airspeed"]) else { continue };
                    e.turbines.push(GtPart {
                        engine: Turbine::new(Params::from_part(p, base), false),
                        air: [air[0], air[1], air[2]],
                        fuel: expr_of(p, "fuel_flow"),
                        starter: expr_of(p, "starter_torque"),
                        offtake: expr_of(p, "offtake"),
                        bleed: expr_of(p, "bleed"),
                        reverser: expr_of(p, "reverser"),
                        output_rpm: expr_of(p, "output_rpm"),
                        s: Slots::of(p, &["thrust", "n1", "n2", "n3", "egt", "tet", "p3", "t3", "p_ip", "fuel_flow", "airflow", "shaft_power", "running", "failed"]),
                    });
                }
                "piston" => {
                    let Some(air) = air_slots(c, p, &["pressure", "temperature"]) else { continue };
                    e.pistons.push(Piston {
                        air: [air[0], air[1]],
                        rated: p.num("rated_power"),
                        rated_rpm: (p.num("rated_rpm") * 60.0).max(1.0),
                        rpm: expr_of(p, "rpm"),
                        throttle: expr_of(p, "throttle"),
                        mixture: expr_of(p, "mixture"),
                        mags: expr_of(p, "magnetos"),
                        fuel_ok: expr_of(p, "fuel_available"),
                        turbo: p.flag("turbocharged"),
                        bsfc: p.num("bsfc") * 0.453_592 / (745.7 * 3600.0),
                        egt: 288.15,
                        cht: 288.15,
                        s: Slots::of(p, &["power", "manifold_pressure", "fuel_flow", "egt", "cht", "running", "failed"]),
                    });
                }
                "propeller" => {
                    let Some(air) = air_slots(c, p, &["pressure", "temperature", "airspeed"]) else { continue };
                    e.props.push(Prop {
                        air: [air[0], air[1], air[2]],
                        power: expr_of(p, "power"),
                        diameter: p.num("diameter"),
                        rated_rpm: (p.num("rated_rpm") * 60.0).max(1.0),
                        rated_power: p.num("rated_power").max(1.0),
                        inertia: p.num("inertia").max(0.01),
                        eta: p.num("efficiency"),
                        governor: expr_of(p, "governor"),
                        feather: expr_of(p, "feather_when"),
                        beta: expr_of(p, "beta"),
                        starter: expr_of(p, "starter_torque"),
                        friction: p.num("friction"),
                        rpm: 0.0,
                        pitch: 1.0,
                        s: Slots::of(p, &["rpm", "thrust", "pitch", "feathered", "failed"]),
                    });
                }
                _ => {}
            }
        }
        (!(e.turbines.is_empty() && e.pistons.is_empty() && e.props.is_empty() && e.forces.is_empty())).then_some(e)
    }
}

fn eval(e: &mut Option<Expr>, slots: &[f64], dt: f64) -> f64 {
    e.as_mut().map_or(0.0, |x| x.eval(slots, dt))
}

impl Domain for Engines {
    fn name(&self) -> &'static str {
        "engines"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        for t in &mut self.turbines {
            let (p0, t0, v0) = (slots[t.air[0]].max(1.0), slots[t.air[1]].max(150.0), slots[t.air[2]]);
            let mach = v0 / (1.4 * 287.05 * t0).sqrt();
            let failed = t.s.on(slots, "failed");
            let flight = Flight {
                p0,
                t0,
                v0,
                mach,
                fuel: if failed { 0.0 } else { eval(&mut t.fuel, slots, dt).max(0.0) },
                bleed: eval(&mut t.bleed, slots, dt).max(0.0),
                output_rpm: eval(&mut t.output_rpm, slots, dt) * 60.0,
            };
            let starter = eval(&mut t.starter, slots, dt);
            let offtake = eval(&mut t.offtake, slots, dt).max(0.0);
            let g = t.engine.step(&flight, starter, offtake, dt);
            let rev = eval(&mut t.reverser, slots, dt).clamp(0.0, 1.0);
            let p = &t.engine.path.p;
            let thrust = g.thrust * (1.0 - rev * (1.0 + p.rev_eff) * p.bpr / (1.0 + p.bpr));
            let n = t.engine.n;
            let (n1, n2, n3) = match p.spools {
                1 => (n[2], 0.0, 0.0),
                2 => (n[0], n[2], 0.0),
                _ => (n[0], n[1], n[2]),
            };
            t.s.put(slots, "thrust", thrust);
            t.s.put(slots, "n1", n1 * 100.0);
            t.s.put(slots, "n2", n2 * 100.0);
            t.s.put(slots, "n3", n3 * 100.0);
            t.s.put(slots, "egt", g.egt);
            t.s.put(slots, "tet", g.t4);
            t.s.put(slots, "p3", g.p3);
            t.s.put(slots, "t3", g.t3);
            t.s.put(slots, "p_ip", g.p_ip);
            t.s.put(slots, "fuel_flow", g.wf);
            t.s.put(slots, "airflow", g.w);
            t.s.put(slots, "shaft_power", g.shaft);
            t.s.flag(slots, "running", g.wf > 0.0 && g.t4 > g.t3 + 100.0);
        }
        for e in &mut self.pistons {
            let (p0, t0) = (slots[e.air[0]].max(1.0), slots[e.air[1]].max(150.0));
            let rpm = eval(&mut e.rpm, slots, dt).max(0.0) * 60.0;
            let thr = eval(&mut e.throttle, slots, dt).clamp(0.0, 1.0);
            let mix = eval(&mut e.mixture, slots, dt).clamp(0.0, 1.0);
            let mags = eval(&mut e.mags, slots, dt).clamp(0.0, 2.0).round();
            let fuel_ok = e.fuel_ok.as_mut().is_none_or(|x| x.yes(slots, dt)) && !e.s.on(slots, "failed");
            let supply = if e.turbo { 101_325.0f64.max(p0) } else { p0 };
            let map = p0 * 0.25 + (supply - p0 * 0.25) * thr;
            let density = (p0 / 101_325.0) * (288.15 / t0);
            let best = 0.55 + 0.45 * if e.turbo { 1.0 } else { density };
            let mix_eff = if mix < 0.15 { 0.0 } else { (1.0 - 2.5 * (mix - best).powi(2)).max(0.0) };
            let mag = match mags as u8 {
                0 => 0.0,
                1 => 0.96,
                _ => 1.0,
            };
            let firing = fuel_ok && mag > 0.0 && rpm > 150.0 && mix_eff > 0.0;
            let power = if firing { e.rated * (map / 101_325.0) * (rpm / e.rated_rpm) * mix_eff * mag } else { 0.0 };
            let richness = (mix / best.max(0.1)).max(0.5);
            let ff = if firing { e.bsfc * power * richness } else { 0.0 };
            let egt_t = if firing { 288.15 + 600.0 * (power / e.rated) * (1.4 - 0.5 * richness).clamp(0.6, 1.2) + 150.0 } else { t0 };
            e.egt += (egt_t - e.egt) * (1.0 - (-dt / 3.0).exp());
            let cht_t = if firing { t0 + 130.0 + 120.0 * power / e.rated } else { t0 };
            e.cht += (cht_t - e.cht) * (1.0 - (-dt / 90.0).exp());
            e.s.put(slots, "power", power);
            e.s.put(slots, "manifold_pressure", map);
            e.s.put(slots, "fuel_flow", ff);
            e.s.put(slots, "egt", e.egt);
            e.s.put(slots, "cht", e.cht);
            e.s.flag(slots, "running", firing && power > 0.02 * e.rated);
        }
        for (e, slot) in &mut self.forces {
            slots[*slot] = eval(e, slots, dt);
        }
        for pr in &mut self.props {
            let (p0, t0, v) = (slots[pr.air[0]].max(1.0), slots[pr.air[1]].max(150.0), slots[pr.air[2]].max(0.0));
            let rho = p0 / (287.05 * t0);
            let shaft = if pr.s.on(slots, "failed") { 0.0 } else { eval(&mut pr.power, slots, dt).max(0.0) };
            let governed = eval(&mut pr.governor, slots, dt) * 60.0;
            let feathered = pr.feather.as_mut().is_some_and(|x| x.yes(slots, dt));
            let beta = eval(&mut pr.beta, slots, dt).clamp(0.0, 1.0);
            let starter = eval(&mut pr.starter, slots, dt);
            if feathered {
                pr.pitch = (pr.pitch + dt * 0.5).min(3.0);
            } else if governed > 0.0 {
                pr.pitch = (pr.pitch + (pr.rpm - governed) / governed * 2.0 * dt).clamp(0.2, 2.5);
            } else {
                pr.pitch = 1.0;
            }
            let pitch = if beta > 0.0 { 0.6 } else { pr.pitch };
            let n_ratio = pr.rpm / pr.rated_rpm;
            let absorbed = if feathered { 0.02 } else { 1.0 } * pr.rated_power * (rho / 1.225) * n_ratio.powi(3) * pitch.powf(1.5);
            let windmill = if feathered { 0.0 } else { 0.5 * rho * v * v * std::f64::consts::PI * (pr.diameter * 0.5).powi(2) * 0.02 * v / (pitch * 40.0).max(1.0) };
            let omega = (pr.rpm * TAU / 60.0).max(2.0);
            let friction = pr.friction * n_ratio.powi(2);
            let net = shaft + windmill + starter * omega - absorbed - friction;
            pr.rpm = (pr.rpm + net / (pr.inertia * omega) * dt * 60.0 / TAU).clamp(0.0, 1.3 * pr.rated_rpm);
            let area = std::f64::consts::PI * (pr.diameter * 0.5).powi(2);
            let p_use = absorbed.min(shaft.max(absorbed));
            let static_t = (2.0 * rho * area * p_use * p_use).powf(1.0 / 3.0) * 0.75;
            let advance = if v > 1.0 { pr.eta * p_use / v } else { static_t };
            let mut thrust = advance.min(static_t.max(1.0));
            if beta > 0.0 {
                thrust = -beta * 0.4 * static_t;
            }
            if feathered || pr.rpm < 10.0 {
                thrust = 0.0;
            }
            pr.s.put(slots, "rpm", pr.rpm / 60.0);
            pr.s.put(slots, "thrust", thrust);
            pr.s.put(slots, "pitch", pr.pitch);
            pr.s.flag(slots, "feathered", feathered);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut p = Packer::default();
        for (e, _) in &self.forces {
            e.iter().for_each(|x| x.state.iter().for_each(|&v| p.put(v)));
        }
        for t in &self.turbines {
            t.engine.n.iter().for_each(|&x| p.put(x));
        }
        for e in &self.pistons {
            p.put(e.egt);
            p.put(e.cht);
        }
        for pr in &self.props {
            p.put(pr.rpm);
            p.put(pr.pitch);
        }
        p.0
    }

    fn restore(&mut self, s: &[f64]) {
        let mut p = Packer(s.to_vec(), 0);
        for (e, _) in &mut self.forces {
            if let Some(x) = e {
                x.state.iter_mut().for_each(|v| *v = p.take());
            }
        }
        for t in &mut self.turbines {
            for x in t.engine.n.iter_mut() {
                *x = p.take();
            }
        }
        for e in &mut self.pistons {
            e.egt = p.take();
            e.cht = p.take();
        }
        for pr in &mut self.props {
            pr.rpm = p.take();
            pr.pitch = p.take();
        }
    }
}

/// One fit target: a share of design thrust, sea-level static, and what was measured there.
#[derive(Clone, Debug)]
pub struct FitPoint {
    pub thrust_share: f64,
    pub fuel_flow: Option<f64>,
    pub egt: Option<f64>,
}

/// Fuel flow and EGT at a thrust share, by bisecting fuel at steady state.
pub fn point(p: &Params, share: f64) -> (f64, f64, f64) {
    let base = Turbine::new(p.clone(), true);
    let target = share * p.design_thrust;
    let (mut lo, mut hi) = (0.0, base.path.d.wf * 1.6);
    let mut g = Gas::default();
    for _ in 0..22 {
        let mid = 0.5 * (lo + hi);
        let mut e = Turbine { path: GasPath { p: base.path.p.clone(), d: base.path.d.clone() }, n: [share.sqrt().max(0.3); 3], last: Gas::default() };
        g = e.settle(&Flight { fuel: mid, ..Flight::static_isa() }, 25.0);
        if g.thrust < target {
            lo = mid
        } else {
            hi = mid
        }
    }
    (g.wf, g.egt, g.thrust)
}

pub fn fit_error(p: &Params, points: &[FitPoint]) -> (f64, Vec<(f64, f64, f64)>) {
    let mut err = 0.0;
    let mut rows = Vec::new();
    for pt in points {
        let (wf, egt, thrust) = point(p, pt.thrust_share);
        if let Some(target) = pt.fuel_flow {
            err += ((wf - target) / target).powi(2);
        }
        if let Some(target) = pt.egt {
            err += ((egt - target) / (target - 288.15).max(50.0)).powi(2);
        }
        rows.push((wf, egt, thrust));
    }
    (err, rows)
}

/// Coordinate search over the named efficiency parameters; returns the tuned set.
pub fn fit(mut p: Params, points: &[FitPoint], tune: &[&str], rounds: usize) -> Params {
    let get = |p: &mut Params, k: &str| -> Option<*mut f64> {
        Some(match k {
            "fan_efficiency" => &mut p.eta_c[0],
            "lpc_efficiency" => &mut p.eta_c[1],
            "ipc_efficiency" => &mut p.eta_c[2],
            "hpc_efficiency" => &mut p.eta_c[3],
            "hpt_efficiency" => &mut p.eta_t[0],
            "ipt_efficiency" => &mut p.eta_t[1],
            "lpt_efficiency" => &mut p.eta_t[2],
            "burner_efficiency" => &mut p.eta_b,
            "cooling" => &mut p.cooling,
            "airflow" => &mut p.airflow,
            _ => return None,
        })
    };
    let mut best = fit_error(&p, points).0;
    let mut step = 0.02;
    for _ in 0..rounds {
        for k in tune {
            for dir in [1.0, -1.0] {
                let mut q = p.clone();
                let Some(ptr) = get(&mut q, k) else { continue };
                unsafe {
                    let scale = if *k == "airflow" { *ptr } else { 1.0 };
                    *ptr = (*ptr + dir * step * scale).clamp(0.0, if *k == "airflow" { f64::MAX } else { 0.99 });
                }
                let e = fit_error(&q, points).0;
                if e < best {
                    best = e;
                    p = q;
                    break;
                }
            }
        }
        step *= 0.6;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turbofan() -> Params {
        Params {
            design_thrust: 120_000.0,
            design_power: 0.0,
            airflow: 400.0,
            bpr: 5.5,
            spools: 2,
            pr: [1.6, 1.4, 1.0, 14.0],
            eta_c: [0.9, 0.89, 0.89, 0.86],
            eta_t: [0.9, 0.91, 0.92],
            eta_b: 0.995,
            loss_b: 0.05,
            lhv: 43.1e6,
            cooling: 0.0,
            rpm: [5000.0, 8000.0, 14000.0],
            inertia: [60.0, 20.0, 8.0],
            fan_gear: 1.0,
            rev_eff: 0.35,
            hpc_map: None,
            fan_map: None,
        }
    }

    #[test]
    fn the_design_point_makes_its_thrust_at_full_speed() {
        let g = GasPath::new(turbofan());
        assert!(g.d.t4 > 1200.0 && g.d.t4 < 2000.0, "T4 {}", g.d.t4);
        let gas = g.eval(&Flight { fuel: g.d.wf, ..Flight::static_isa() }, [1.0; 3]);
        assert!((gas.thrust - 120_000.0).abs() / 120_000.0 < 0.02, "{}", gas.thrust);
        assert!(gas.surplus.iter().all(|s| s.abs() < 0.03 * g.d.wf * 43.1e6), "balanced spools {:?}", gas.surplus);
    }

    #[test]
    fn the_engine_settles_and_less_fuel_means_less_thrust() {
        let mut e = Turbine::new(turbofan(), true);
        let wf = e.path.d.wf;
        let full = e.settle(&Flight { fuel: wf, ..Flight::static_isa() }, 30.0);
        assert!((full.thrust - 120_000.0).abs() / 120_000.0 < 0.05, "{}", full.thrust);
        let part = e.settle(&Flight { fuel: 0.4 * wf, ..Flight::static_isa() }, 40.0);
        assert!(part.thrust < 0.6 * full.thrust && part.thrust > 0.0, "{}", part.thrust);
        assert!(e.n[0] < 1.0 && e.n[2] < 1.0);
        let off = e.settle(&Flight { fuel: 0.0, ..Flight::static_isa() }, 60.0);
        assert!(off.thrust < 5_000.0, "{}", off.thrust);
    }

    #[test]
    fn the_fit_moves_fuel_flow_towards_the_targets() {
        let p = turbofan();
        let (wf, _, _) = point(&p, 1.0);
        let pts = vec![FitPoint { thrust_share: 1.0, fuel_flow: Some(wf * 1.08), egt: None }];
        let before = fit_error(&p, &pts).0;
        let q = fit(p, &pts, &["hpc_efficiency", "hpt_efficiency"], 4);
        let after = fit_error(&q, &pts).0;
        assert!(after < before * 0.5, "{before} -> {after}");
    }
}

/// Every `output.force`: its value slot, position, unit direction, scale, and whether it replaces X-Plane's engines.
pub fn forces(c: &Checked) -> Vec<(usize, [f64; 3], [f64; 3], f64, bool)> {
    c.parts
        .iter()
        .filter(|p| p.domain == "output" && p.kind == "force")
        .map(|p| {
            let (pt, yw) = (p.num("pitch"), p.num("yaw"));
            let dir = [yw.sin() * pt.cos(), pt.sin(), -yw.cos() * pt.cos()];
            (crate::catalog::out(p, "force"), [p.num("x"), p.num("y"), p.num("z")], dir, p.num("scale"), p.flag("replaces_engines"))
        })
        .collect()
}

/// Sums forces into X-Plane's `[fside, fnrml, faxil, L, M, N]` (X right, Y up, Z aft; right roll, nose up, nose right).
pub fn sum_forces(items: &[([f64; 3], [f64; 3], f64)]) -> [f64; 6] {
    let mut out = [0.0; 6];
    for (r, d, f) in items {
        let v = [d[0] * f, d[1] * f, d[2] * f];
        let tau = [r[1] * v[2] - r[2] * v[1], r[2] * v[0] - r[0] * v[2], r[0] * v[1] - r[1] * v[0]];
        out[0] += v[0];
        out[1] += v[1];
        out[2] += v[2];
        out[3] -= tau[2];
        out[4] += tau[0];
        out[5] -= tau[1];
    }
    out
}
