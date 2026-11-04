//! FADEC and APU controllers.

use crate::engine::{GasPath, Params};
use crate::util::{Packer, Slots};
use crate::{expr_of, Domain, Expr};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const MF: Type = Type::Num(Dim::MASS_FLOW);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub static SPECS: &[PartSpec] = &[
    PartSpec {
        domain: "fadec",
        kind: "controller",
        help: "An engine's FADEC: lever to rating, governing, limits, start and relight, reverser, channels.",
        fields: &[
            req("engine", Kind::Ref(&["engine.gas_turbine"]), "the engine it controls"),
            req("lever", Kind::Expr(N), "thrust lever: 0 idle to 1 full, negative for reverse"),
            opt("governs", Kind::Text, "n1", "n1, or n2 for the core spool (n3 on three spools)"),
            opt("idle", Kind::Expr(N), "22", "idle setting, percent"),
            opt("max", Kind::Expr(N), "100", "rating at full lever, percent; a table() for real ratings"),
            opt("max_reverse", Kind::Expr(N), "70", "setting at full reverse, percent"),
            opt("master", WHEN, "true", "fuel master on"),
            opt("start", WHEN, "false", "start selected"),
            opt("autothrust_active", WHEN, "false", "autothrust in control"),
            opt("autothrust_target", Kind::Expr(N), "0", "autothrust setting, percent, capped by the lever"),
            opt("acceleration", Kind::Number, "0.3", "fuel rise per second, share of design fuel flow"),
            opt("deceleration", Kind::Number, "0.5", "fuel fall per second, share of design fuel flow"),
            opt("egt_limit", Kind::Quantity(Dim::TEMPERATURE), "1250 K", "running EGT limit"),
            opt("speed_limit", Kind::Number, "104", "overspeed protection, percent"),
            opt("fuel_on_at", Kind::Number, "18", "core speed for fuel during a start, percent"),
            opt("starter_cutout", Kind::Number, "50", "core speed the starter drops out at, percent"),
            opt("light_off_time", Kind::Quantity(Dim::TIME), "15 s", "no light-off by then aborts the start"),
            opt("start_egt_limit", Kind::Quantity(Dim::TEMPERATURE), "1050 K", "start EGT limit"),
            opt("hung_time", Kind::Quantity(Dim::TIME), "60 s", "time to reach starter cutout before a hung start aborts"),
            opt("reverser_time", Kind::Quantity(Dim::TIME), "2 s", "reverser travel time"),
            opt("start_fuel", Kind::Number, "0.06", "start fuel at light-off, share of design fuel flow; it rises fourfold by starter cutout"),
            opt("channel_a_failed", WHEN, "false", "channel A lost"),
            opt("channel_b_failed", WHEN, "false", "channel B lost"),
            opt("auto_relight", Kind::Bool, "true", "ignition comes on by itself when the engine flames out while running"),
        ],
        outputs: &[
            ("fuel_command", MF),
            ("starter", B),
            ("ignition", B),
            ("target", N),
            ("state", N),
            ("aborted", B),
            ("channel", N),
            ("fault", B),
            ("limiting", B),
            ("reverser", N),
            ("failed", B),
            ("flameout", B),
            ("egt_exceedance", T),
        ],
    },
];

const OFF: f64 = 0.0;
const CRANK: f64 = 1.0;
const LIGHT: f64 = 2.0;
const RUN: f64 = 3.0;
const ABORT: f64 = 4.0;

struct Fadec {
    engine: [usize; 4],
    governs_n2: bool,
    wf_d: f64,
    e: [Option<Expr>; 10],
    k: [f64; 11],
    state: f64,
    wf: f64,
    integral: f64,
    timer: f64,
    egt_start: f64,
    rev: f64,
    relight: bool,
    exceed: f64,
    s: Slots,
}


pub struct Control {
    fadecs: Vec<Fadec>,
    apus: Vec<crate::apu::Apu>,
}

fn slot(c: &Checked, p: &CheckedPart, field: &str, out: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{out}", c.parts[i].id)).copied())
}

fn ev(e: &mut Option<Expr>, s: &[f64], dt: f64) -> f64 {
    e.as_mut().map_or(0.0, |x| x.eval(s, dt))
}

fn yes(e: &mut Option<Expr>, s: &[f64], dt: f64) -> bool {
    ev(e, s, dt) != 0.0
}

impl Control {
    pub fn new(c: &Checked, base: &std::path::Path) -> Option<Self> {
        let mut ctl = Control { fadecs: vec![], apus: vec![] };
        for p in &c.parts {
            match (p.domain, p.kind) {
                ("fadec", "controller") => {
                    let Some(eng) = p.part("engine").map(|i| &c.parts[i]) else { continue };
                    let core = if eng.num("spools") >= 3.0 { "n3" } else { "n2" };
                    let (Some(n1), Some(n2), Some(egt), Some(run)) = (slot(c, p, "engine", "n1"), slot(c, p, "engine", core), slot(c, p, "engine", "egt"), slot(c, p, "engine", "running")) else { continue };
                    let wf_d = GasPath::new(Params::from_part(eng, base)).d.wf.max(1e-4);
                    let e = ["lever", "idle", "max", "max_reverse", "master", "start", "autothrust_active", "autothrust_target", "channel_a_failed", "channel_b_failed"].map(|k| expr_of(p, k));
                    let k = [p.num("acceleration"), p.num("deceleration"), p.num("egt_limit"), p.num("speed_limit"), p.num("fuel_on_at"), p.num("starter_cutout"), p.num("light_off_time"), p.num("start_egt_limit"), p.num("hung_time"), p.num("reverser_time").max(0.1), p.num("start_fuel")];
                    ctl.fadecs.push(Fadec {
                        engine: [n1, n2, egt, run],
                        governs_n2: p.text("governs").eq_ignore_ascii_case("n2"),
                        wf_d,
                        e,
                        k,
                        state: OFF,
                        wf: 0.0,
                        integral: 0.0,
                        timer: 0.0,
                        egt_start: 0.0,
                        rev: 0.0,
                        relight: p.flag("auto_relight"),
                        exceed: 0.0,
                        s: Slots::of(p, &["fuel_command", "starter", "ignition", "target", "state", "aborted", "channel", "fault", "limiting", "reverser", "failed", "flameout", "egt_exceedance"]),
                    });
                }
                ("apu", "unit") => ctl.apus.push(crate::apu::Apu::new(c, p)),
                _ => {}
            }
        }
        (!(ctl.fadecs.is_empty() && ctl.apus.is_empty())).then_some(ctl)
    }
}

impl Domain for Control {
    fn name(&self) -> &'static str {
        "control"
    }

    fn step(&mut self, slots: &mut [f64], dt: f64) {
        for f in &mut self.fadecs {
            let n1 = slots[f.engine[0]];
            let n2 = slots[f.engine[1]];
            let egt = slots[f.engine[2]];
            let lever = ev(&mut f.e[0], slots, dt).clamp(-1.0, 1.0);
            let idle = ev(&mut f.e[1], slots, dt);
            let max = ev(&mut f.e[2], slots, dt);
            let max_rev = ev(&mut f.e[3], slots, dt);
            let master = yes(&mut f.e[4], slots, dt);
            let start = yes(&mut f.e[5], slots, dt);
            let at_on = yes(&mut f.e[6], slots, dt);
            let at = ev(&mut f.e[7], slots, dt);
            let (a_bad, b_bad) = (yes(&mut f.e[8], slots, dt), yes(&mut f.e[9], slots, dt));
            let failed = f.s.on(slots, "failed") || (a_bad && b_bad);
            let channel = if failed { 0.0 } else if a_bad { 2.0 } else { 1.0 };
            let [acc, dec, egt_lim, n_lim, fuel_on, cutout, light_t, start_egt, hung_t, rev_t, start_fuel] = f.k;
            let reverse = lever < 0.0;
            f.rev = (f.rev + if reverse { dt / rev_t } else { -dt / rev_t }).clamp(0.0, 1.0);
            let mut target = if reverse { idle + (-lever) * (max_rev - idle) } else { idle + lever * (max - idle) };
            if at_on && !reverse {
                target = at.clamp(idle, target.max(idle));
            }
            if reverse && f.rev < 1.0 {
                target = idle;
            }
            let mut starter = false;
            let mut ignition = false;
            let mut limiting = false;
            if !master || failed {
                f.state = if f.state == ABORT && !start { OFF } else if master { f.state } else { OFF };
                f.wf = if failed && master { f.wf } else { 0.0 };
            }
            if master && !failed {
                match f.state as u8 {
                    0 => {
                        if start && n2 < cutout {
                            f.state = CRANK;
                            f.timer = 0.0;
                        } else if n2 >= cutout {
                            f.state = RUN;
                            f.wf = 0.05 * f.wf_d;
                        }
                    }
                    1 => {
                        starter = true;
                        f.timer += dt;
                        if n2 >= fuel_on {
                            f.state = LIGHT;
                            f.egt_start = egt;
                            f.timer = 0.0;
                        } else if f.timer > hung_t {
                            f.state = ABORT;
                        }
                    }
                    2 => {
                        starter = true;
                        ignition = true;
                        f.timer += dt;
                        f.wf = start_fuel * f.wf_d * (1.0 + 3.0 * ((n2 - fuel_on) / (cutout - fuel_on).max(1.0)).clamp(0.0, 1.0));
                        let lit = egt > f.egt_start + 100.0;
                        if egt > start_egt || (!lit && f.timer > light_t) || f.timer > hung_t {
                            f.state = ABORT;
                            f.wf = 0.0;
                        } else if n2 >= cutout {
                            f.state = RUN;
                            f.integral = 0.0;
                        }
                    }
                    3 => {
                        let n = if f.governs_n2 { n2 } else { n1 };
                        let err = target - n;
                        f.integral = (f.integral + err * dt).clamp(-150.0, 150.0);
                        let want = f.wf_d * ((target / 100.0).max(0.0).powi(3) + 0.01 * err + 0.004 * f.integral);
                        let mut next = want.clamp(f.wf - dec * f.wf_d * dt, f.wf + acc * f.wf_d * dt);
                        if egt > egt_lim || n1 > n_lim || n2 > n_lim {
                            next = next.min(f.wf * (1.0 - 0.5 * dt));
                            limiting = true;
                        }
                        f.wf = next.clamp(0.02 * f.wf_d, 1.5 * f.wf_d);
                        if n2 < 0.5 * fuel_on {
                            f.state = OFF;
                            f.wf = 0.0;
                        }
                        if slots[f.engine[3]] == 0.0 && f.relight {
                            ignition = true;
                        }
                    }
                    _ => {
                        f.wf = 0.0;
                        starter = start && n2 < cutout;
                        if !start {
                            f.state = OFF;
                        }
                    }
                }
            }
            f.s.put(slots, "fuel_command", f.wf);
            f.s.flag(slots, "starter", starter);
            f.s.flag(slots, "ignition", ignition || (f.state == RUN && start));
            f.s.put(slots, "target", target);
            f.s.put(slots, "state", f.state);
            f.s.flag(slots, "aborted", f.state == ABORT);
            f.s.put(slots, "channel", channel);
            f.s.flag(slots, "fault", a_bad || b_bad);
            f.s.flag(slots, "limiting", limiting);
            f.s.put(slots, "reverser", f.rev);
            f.s.flag(slots, "flameout", f.state == RUN && slots[f.engine[3]] == 0.0);
            if f.state == RUN && egt > egt_lim {
                f.exceed = f.exceed.max(egt - egt_lim);
            }
            f.s.put(slots, "egt_exceedance", f.exceed);
        }
        for a in &mut self.apus {
            a.step(slots, dt);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut p = Packer::default();
        for f in &self.fadecs {
            for v in [f.state, f.wf, f.integral, f.timer, f.egt_start, f.rev, f.exceed] {
                p.put(v);
            }
        }
        let mut v = Vec::new();
        self.apus.iter().for_each(|a| a.state(&mut v));
        v.into_iter().for_each(|x| p.put(x));
        p.0
    }

    fn restore(&mut self, s: &[f64]) {
        let mut p = Packer(s.to_vec(), 0);
        for f in &mut self.fadecs {
            f.state = p.take();
            f.wf = p.take();
            f.integral = p.take();
            f.timer = p.take();
            f.egt_start = p.take();
            f.rev = p.take();
            f.exceed = p.take();
        }
        let rest: Vec<f64> = (0..).map_while(|_| (p.1 < p.0.len()).then(|| p.take())).collect();
        let mut it = rest.into_iter();
        self.apus.iter_mut().for_each(|a| a.restore(&mut it));
    }
}
