//! Engine and APU controllers: dual-channel FADECs with their start sequencers, governors, limiters and
//! protections, and the APU's control unit.

mod fadec;
mod sensors;

use crate::engine::{GasPath, Params};
use crate::util::Slots;
use crate::{expr_of, Domain};
use easyacft_core::Dim;
use easyacft_lang::schema::{opt, req, Kind, PartSpec};
use easyacft_lang::{Checked, CheckedPart, Type};
use fadec::Fadec;

const B: Type = Type::Bool;
const N: Type = Type::Num(Dim::NONE);
const T: Type = Type::Num(Dim::TEMPERATURE);
const MF: Type = Type::Num(Dim::MASS_FLOW);
const WHEN: Kind = Kind::Expr(Type::Bool);

pub static SPECS: &[PartSpec] = &[PartSpec {
    domain: "fadec",
    kind: "controller",
    help: "An engine's FADEC: two channels on their own probes and power, an automatic start that aborts hot, hung and failed starts, dry-motors and tries again inside the starter's duty cycle, windmill relight; a governor on N1, or on the core in alternate mode, under acceleration and deceleration schedules and EGT, speed and burner pressure limiters; approach idle, derate, autothrust, reverser interlocks, thrust control malfunction and overspeed shutdown.",
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
        opt("auto_relight", Kind::Bool, "true", "ignition comes on by itself when the engine flames out while running, and a windmilling engine relights when its master comes on in flight"),
        opt("on_ground", WHEN, "true", "weight on wheels: reverse allowed, approach idle and windmill relight not"),
        opt("approach_idle", WHEN, "false", "approach idle wanted, as with flaps out, so the engine spools up fast for a go-around"),
        opt("approach_idle_setting", Kind::Number, "30", "approach idle, percent"),
        opt("derate", Kind::Expr(N), "0", "percent taken off the rating, as for a flexible take-off"),
        opt("channel_a_powered", WHEN, "true", "aircraft power to channel A"),
        opt("channel_b_powered", WHEN, "true", "aircraft power to channel B"),
        opt("alternator_at", Kind::Number, "12", "core speed at which the engine's own alternator powers both channels, percent"),
        opt("n1_sensor_failed", Kind::Expr(N), "0", "N1 probes lost: 1 channel A's, 2 channel B's, 3 both, which puts it in alternate mode"),
        opt("n2_sensor_failed", Kind::Expr(N), "0", "core speed probes lost, the same way"),
        opt("egt_sensor_failed", Kind::Expr(N), "0", "EGT probes lost, the same way; with both gone the EGT limiter is blind"),
        opt("reverser_unlocked_when", WHEN, "false", "a reverser unlocked in flight: it forces idle"),
        opt("manual_start", WHEN, "false", "manual start: fuel and ignition earlier, no automatic look-ahead abort"),
        opt("auto_start", Kind::Bool, "true", "automatic start: aborts, dry motoring and another attempt"),
        opt("shutdown_speed", Kind::Number, "0", "overspeed shutdown, percent; 0 for none"),
        opt("max_attempts", Kind::Number, "2", "automatic start attempts before it gives up"),
        opt("dry_motor_time", Kind::Quantity(Dim::TIME), "30 s", "dry motoring between attempts"),
        opt("starter_limit", Kind::Quantity(Dim::TIME), "300 s", "cranking the starter takes before it must cool"),
        opt("windmill_relight", Kind::Number, "12", "core speed a windmilling engine relights from without its starter, percent"),
        opt("p3_limit", Kind::Quantity(Dim::PRESSURE), "0 Pa", "burner pressure limit; 0 for none"),
        opt("relight_time", Kind::Quantity(Dim::TIME), "30 s", "how long a windmill relight is given"),
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
        ("limiter", N),
        ("mode", N),
        ("ignition_a", B),
        ("ignition_b", B),
        ("attempts", N),
        ("starter_overheat", B),
        ("approach_idle_on", B),
        ("thrust_malfunction", B),
        ("shutdown", B),
        ("n1_sensed", N),
        ("egt_sensed", T),
        ("reverser_unlocked", B),
        ("fuel_air", N),
    ],
}];

const OUT_NAMES: [&str; 26] = [
    "fuel_command", "starter", "ignition", "target", "state", "aborted", "channel", "fault", "limiting", "reverser", "failed", "flameout", "egt_exceedance", "limiter", "mode", "ignition_a", "ignition_b", "attempts", "starter_overheat", "approach_idle_on", "thrust_malfunction", "shutdown", "n1_sensed", "egt_sensed", "reverser_unlocked", "fuel_air",
];

pub struct Control {
    fadecs: Vec<Fadec>,
    apus: Vec<crate::apu::Apu>,
}

fn slot(c: &Checked, p: &CheckedPart, field: &str, out: &str) -> Option<usize> {
    p.part(field).and_then(|i| c.slots.index.get(&format!("{}.{out}", c.parts[i].id)).copied())
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
                    let gp = GasPath::new(Params::from_part(eng, base));
                    let wf_d = gp.d.wf.max(1e-4);
                    let p3_d = gp.d.p4 / 0.95;
                    let e = fadec::EXPRS.map(|k| expr_of(p, k));
                    let mut k = fadec::NUMS.map(|n| p.num(n));
                    k[9] = k[9].max(0.1);
                    let governs_n2 = p.text("governs").eq_ignore_ascii_case("n2");
                    ctl.fadecs.push(Fadec::new([n1, n2, egt, run], slot(c, p, "engine", "p3"), governs_n2, wf_d, p3_d, e, k, p.flag("auto_relight"), p.flag("auto_start"), Slots::of(p, &OUT_NAMES)));
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
            f.step(slots, dt);
        }
        for a in &mut self.apus {
            a.step(slots, dt);
        }
    }

    fn state(&self) -> Vec<f64> {
        let mut v = Vec::new();
        for f in &self.fadecs {
            f.state(&mut v);
        }
        self.apus.iter().for_each(|a| a.state(&mut v));
        v
    }

    fn restore(&mut self, s: &[f64]) {
        let mut it = s.iter().copied();
        for f in &mut self.fadecs {
            f.restore(&mut it);
        }
        self.apus.iter_mut().for_each(|a| a.restore(&mut it));
    }
}
