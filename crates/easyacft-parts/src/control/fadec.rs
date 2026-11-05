//! An engine's FADEC. Two channels, one active and one in standby, powered by the aircraft until the engine's
//! own alternator takes over, each reading the engine through its own probes. The start sequencer cranks,
//! lights and accelerates to starter cutout, aborts a hot, hung or failed start, dry-motors to clear fuel and
//! tries again, keeps the starter inside its duty cycle, alternates igniters, and relights a windmilling engine.
//! Running, the governor holds the thrust lever's N1 (or core speed in alternate mode when N1 is lost) with a
//! min-select of limiters: fuel-to-pressure acceleration and deceleration schedules, EGT, overspeed and burner
//! pressure. Approach idle, derate, autothrust, reverser interlocks, thrust control malfunction and overspeed
//! shutdown protection.

use super::sensors::{lost, Probe};
use crate::util::Slots;
use crate::Expr;

pub const OFF: f64 = 0.0;
pub const CRANK: f64 = 1.0;
pub const LIGHT: f64 = 2.0;
pub const RUN: f64 = 3.0;
pub const ABORT: f64 = 4.0;
pub const DRY: f64 = 5.0;
pub const RELIGHT: f64 = 6.0;

/// Expressions a FADEC evaluates, in order.
pub const EXPRS: [&str; 20] = [
    "lever", "idle", "max", "max_reverse", "master", "start", "autothrust_active", "autothrust_target", "channel_a_failed", "channel_b_failed",
    "on_ground", "approach_idle", "derate", "channel_a_powered", "channel_b_powered", "n1_sensor_failed", "n2_sensor_failed", "egt_sensor_failed", "reverser_unlocked_when", "manual_start",
];

/// Numbers from the part, in order.
pub const NUMS: [&str; 20] = [
    "acceleration", "deceleration", "egt_limit", "speed_limit", "fuel_on_at", "starter_cutout", "light_off_time", "start_egt_limit", "hung_time", "reverser_time",
    "start_fuel", "approach_idle_setting", "shutdown_speed", "alternator_at", "max_attempts", "dry_motor_time", "starter_limit", "windmill_relight", "p3_limit", "relight_time",
];

pub struct Fadec {
    /// Slots of the engine's fan speed, core speed, EGT, running flag and burner pressure.
    pub engine: [usize; 4],
    pub p3: Option<usize>,
    pub governs_n2: bool,
    pub wf_d: f64,
    /// Design fuel over burner pressure, kg/s per Pa.
    pub wf_p3: f64,
    pub e: [Option<Expr>; 20],
    pub k: [f64; 20],
    pub relight: bool,
    pub auto_start: bool,
    pub state: f64,
    pub wf: f64,
    integral: f64,
    timer: f64,
    egt_start: f64,
    rev: f64,
    exceed: f64,
    attempts: f64,
    starter_heat: f64,
    starter_hot: bool,
    active: f64,
    probes: [Probe; 3],
    n2_mode_ratio: f64,
    tcm_t: f64,
    shutdown: bool,
    egt_last: f64,
    pub s: Slots,
}

fn ev(e: &mut Option<Expr>, s: &[f64], dt: f64, or: f64) -> f64 {
    e.as_mut().map_or(or, |x| x.eval(s, dt))
}

impl Fadec {
    #[allow(clippy::too_many_arguments)]
    pub fn new(engine: [usize; 4], p3: Option<usize>, governs_n2: bool, wf_d: f64, p3_d: f64, e: [Option<Expr>; 20], k: [f64; 20], relight: bool, auto_start: bool, s: Slots) -> Self {
        Fadec {
            engine,
            p3,
            governs_n2,
            wf_d,
            wf_p3: wf_d / p3_d.max(1e4),
            e,
            k,
            relight,
            auto_start,
            state: OFF,
            wf: 0.0,
            integral: 0.0,
            timer: 0.0,
            egt_start: 0.0,
            rev: 0.0,
            exceed: 0.0,
            attempts: 0.0,
            starter_heat: 0.0,
            starter_hot: false,
            active: 1.0,
            probes: [Probe::new(0.04, 0.05, 0.05, 1.5), Probe::new(0.04, 0.05, 0.05, 1.5), Probe::new(0.6, 1.0, 1.5, 25.0)],
            n2_mode_ratio: 0.0,
            tcm_t: 0.0,
            shutdown: false,
            egt_last: 0.0,
            s,
        }
    }

    pub fn step(&mut self, slots: &mut [f64], dt: f64) {
        let truth_n1 = slots[self.engine[0]];
        let truth_n2 = slots[self.engine[1]];
        let truth_egt = slots[self.engine[2]];
        let running = slots[self.engine[3]] != 0.0;
        let p3 = self.p3.map(|i| slots[i]);
        let e = &mut self.e;
        let lever = ev(&mut e[0], slots, dt, 0.0).clamp(-1.0, 1.0);
        let idle = ev(&mut e[1], slots, dt, 22.0);
        let max = ev(&mut e[2], slots, dt, 100.0);
        let max_rev = ev(&mut e[3], slots, dt, 70.0);
        let master = ev(&mut e[4], slots, dt, 1.0) != 0.0;
        let start = ev(&mut e[5], slots, dt, 0.0) != 0.0;
        let at_on = ev(&mut e[6], slots, dt, 0.0) != 0.0;
        let at = ev(&mut e[7], slots, dt, 0.0);
        let (a_bad, b_bad) = (ev(&mut e[8], slots, dt, 0.0) != 0.0, ev(&mut e[9], slots, dt, 0.0) != 0.0);
        let on_ground = ev(&mut e[10], slots, dt, 1.0) != 0.0;
        let approach = ev(&mut e[11], slots, dt, 0.0) != 0.0;
        let derate = ev(&mut e[12], slots, dt, 0.0).clamp(0.0, 40.0);
        let (a_pwr, b_pwr) = (ev(&mut e[13], slots, dt, 1.0) != 0.0, ev(&mut e[14], slots, dt, 1.0) != 0.0);
        let lost_codes = [ev(&mut e[15], slots, dt, 0.0), ev(&mut e[16], slots, dt, 0.0), ev(&mut e[17], slots, dt, 0.0)];
        let rev_unlocked = ev(&mut e[18], slots, dt, 0.0) != 0.0;
        let manual = ev(&mut e[19], slots, dt, 0.0) != 0.0;
        let [acc, dec, egt_lim, n_lim, fuel_on, cutout, light_t, start_egt, hung_t, rev_t, start_fuel, appr_set, shut_n, alt_at, max_att, dry_t, starter_lim, windmill, p3_lim, relight_t] = self.k;
        // Power: the engine's alternator feeds both channels once the core turns fast enough.
        let alternator = truth_n2 >= alt_at;
        let ch = [!a_bad && (a_pwr || alternator), !b_bad && (b_pwr || alternator)];
        let failed = self.s.on(slots, "failed") || (!ch[0] && !ch[1]);
        if !ch[(self.active as usize).saturating_sub(1).min(1)] {
            self.active = if ch[0] { 1.0 } else { 2.0 };
        }
        let channel = if failed { 0.0 } else { self.active };
        let n1r = self.probes[0].sense(truth_n1, lost(lost_codes[0]), dt);
        let n2r = self.probes[1].sense(truth_n2, lost(lost_codes[1]), dt);
        let egtr = self.probes[2].sense(truth_egt, lost(lost_codes[2]), dt);
        let n2 = if n2r.valid { n2r.value } else { truth_n2 };
        let egt = if egtr.valid { egtr.value } else { 0.0 };
        let alternate = !n1r.valid && !self.governs_n2;
        if !alternate && n1r.valid && n2 > 1.0 {
            self.n2_mode_ratio = n2r.value / n1r.value.max(1.0);
        }
        let n1 = if n1r.valid { n1r.value } else { truth_n1 };
        // Reverser: travels on its actuators; the interlock keeps it stowed in the air.
        let reverse = lever < 0.0 && on_ground;
        self.rev = (self.rev + if reverse { dt / rev_t } else { -dt / rev_t }).clamp(0.0, 1.0);
        let mut target = if lever < 0.0 { idle + (-lever) * (max_rev - idle) } else { idle + lever * (max - derate - idle) };
        if at_on && lever >= 0.0 {
            target = at.clamp(idle, target.max(idle));
        }
        let idle_now = if approach && !on_ground { idle.max(appr_set) } else { idle };
        target = target.max(idle_now);
        if lever < 0.0 && (self.rev < 1.0 || !on_ground) {
            target = idle_now;
        }
        if rev_unlocked && !on_ground {
            // Reverser unlocked in flight: forced to idle.
            target = idle_now;
        }
        let mut starter = false;
        let mut ignition = false;
        let mut limiter = 0.0;
        // Starter duty: heats while it cranks, cools at a third of that rate.
        if !master || failed {
            self.state = if self.state == ABORT && !start { OFF } else if master { self.state } else { OFF };
            self.wf = if failed && master { self.wf } else { 0.0 };
            if !master {
                self.attempts = 0.0;
                self.shutdown = false;
            }
        }
        if master && !failed && !self.shutdown {
            match self.state as u8 {
                0 => {
                    if start && n2 < cutout && !self.starter_hot {
                        self.state = CRANK;
                        self.timer = 0.0;
                        self.attempts += 1.0;
                    } else if n2 >= cutout {
                        self.state = RUN;
                        self.wf = 0.05 * self.wf_d;
                    } else if !on_ground && self.relight && n2 >= windmill && !start {
                        self.state = RELIGHT;
                        self.timer = 0.0;
                        self.egt_start = egt;
                    }
                }
                1 => {
                    starter = true;
                    self.timer += dt;
                    if n2 >= fuel_on || (manual && n2 >= 0.6 * fuel_on) {
                        self.state = LIGHT;
                        self.egt_start = egt;
                        self.egt_last = egt;
                        self.timer = 0.0;
                    } else if self.timer > hung_t || self.starter_hot {
                        self.abort();
                    }
                }
                2 => {
                    starter = true;
                    ignition = true;
                    self.timer += dt;
                    self.wf = start_fuel * self.wf_d * (1.0 + 3.0 * ((n2 - fuel_on) / (cutout - fuel_on).max(1.0)).clamp(0.0, 1.0));
                    let lit = egt > self.egt_start + 100.0;
                    // Hot start protection looks ahead along the EGT's rise.
                    let rise = (egt - self.egt_last) / dt.max(1e-6);
                    self.egt_last = egt;
                    // Close to the limit and still climbing fast, it aborts before the limit is crossed.
                    let predicted = egt + rise.max(0.0) * 0.5;
                    let hot = egt > start_egt || (self.auto_start && !manual && egt > start_egt - 150.0 && predicted > start_egt);
                    if hot || (!lit && self.timer > light_t) || self.timer > hung_t {
                        self.abort();
                    } else if n2 >= cutout {
                        self.state = RUN;
                        self.integral = 0.0;
                        self.attempts = 0.0;
                    }
                }
                3 => {
                    let n = if self.governs_n2 { n2 } else if alternate { n2 / self.n2_mode_ratio.max(0.5) } else { n1 };
                    let err = target - n;
                    self.integral = (self.integral + err * dt).clamp(-150.0, 150.0);
                    let want = self.wf_d * ((target / 100.0).max(0.0).powi(3) + 0.01 * err + 0.004 * self.integral);
                    let mut lo = self.wf - dec * self.wf_d * dt;
                    let mut hi = self.wf + acc * self.wf_d * dt;
                    // Fuel-to-burner-pressure schedules keep the core off its surge and lean blowout lines.
                    if let Some(p3) = p3.filter(|p| *p > 1e4) {
                        let n2s = (n2 / 100.0).clamp(0.3, 1.1);
                        hi = hi.min(self.wf_p3 * (2.0 + 3.0 * (1.0 - n2s)) * p3);
                        lo = lo.max(self.wf_p3 * 0.25 * p3);
                    }
                    let mut next = want.clamp(lo.min(hi), hi);
                    if next >= hi - 1e-12 && want > hi {
                        limiter = 1.0;
                    } else if next <= lo + 1e-12 && want < lo {
                        limiter = 2.0;
                    }
                    if egtr.valid && egt > egt_lim {
                        next = next.min(self.wf * (1.0 - 0.5 * dt * (1.0 + (egt - egt_lim) / 50.0)));
                        limiter = 3.0;
                    }
                    if n1 > n_lim {
                        next = next.min(self.wf * (1.0 - 0.5 * dt));
                        limiter = 4.0;
                    }
                    if n2 > n_lim {
                        next = next.min(self.wf * (1.0 - 0.5 * dt));
                        limiter = 5.0;
                    }
                    if p3_lim > 0.0 && p3.is_some_and(|p| p > p3_lim) {
                        next = next.min(self.wf * (1.0 - 0.5 * dt));
                        limiter = 6.0;
                    }
                    self.wf = next.clamp(0.02 * self.wf_d, 1.5 * self.wf_d);
                    // Thrust control malfunction: speed running away from the lever with nothing limiting it.
                    let away = (n - target).abs() > 10.0 && (n - target).signum() == (self.wf - want).signum() && limiter == 0.0;
                    self.tcm_t = if away && on_ground { self.tcm_t + dt } else { 0.0 };
                    if shut_n > 0.0 && (truth_n1 > shut_n || truth_n2 > shut_n) {
                        self.shutdown = true;
                    }
                    if n2 < 0.5 * fuel_on {
                        self.state = OFF;
                        self.wf = 0.0;
                    }
                    if !running && self.relight {
                        ignition = true;
                    }
                }
                4 => {
                    self.wf = 0.0;
                    starter = start && n2 < cutout && !self.starter_hot;
                    if !start {
                        self.state = OFF;
                        self.attempts = 0.0;
                    } else if self.auto_start && self.attempts < max_att && !self.starter_hot {
                        self.state = DRY;
                        self.timer = 0.0;
                    }
                }
                5 => {
                    // Dry motoring blows the unburnt fuel out before the next attempt.
                    starter = true;
                    self.wf = 0.0;
                    self.timer += dt;
                    if self.timer >= dry_t || self.starter_hot {
                        self.state = if self.starter_hot { ABORT } else { CRANK };
                        self.timer = 0.0;
                        self.attempts += 1.0;
                    }
                }
                _ => {
                    ignition = true;
                    self.timer += dt;
                    self.wf = start_fuel * self.wf_d * 1.5;
                    if egt > self.egt_start + 100.0 || running {
                        self.state = RUN;
                        self.integral = 0.0;
                    } else if self.timer > relight_t || n2 < 0.5 * windmill {
                        self.state = OFF;
                        self.wf = 0.0;
                    }
                }
            }
        }
        if self.shutdown {
            self.wf = 0.0;
            self.state = OFF;
        }
        self.starter_heat = if starter { self.starter_heat + dt } else { (self.starter_heat - dt / 3.0).max(0.0) };
        self.starter_hot = if self.starter_hot { self.starter_heat > 0.5 * starter_lim } else { self.starter_heat > starter_lim };
        let odd = (self.attempts as i64) % 2 == 1;
        let ign_on = ignition || (self.state == RUN && start);
        let both = !on_ground || self.state == RELIGHT || manual;
        let s = &self.s;
        s.put(slots, "fuel_command", self.wf);
        s.flag(slots, "starter", starter);
        s.flag(slots, "ignition", ign_on);
        s.put(slots, "target", target);
        s.put(slots, "state", self.state);
        s.flag(slots, "aborted", self.state == ABORT);
        s.put(slots, "channel", channel);
        s.flag(slots, "fault", a_bad || b_bad || n1r.disagree || n2r.disagree || egtr.disagree || !n1r.valid || !egtr.valid);
        s.flag(slots, "limiting", limiter >= 3.0);
        s.put(slots, "reverser", self.rev);
        s.flag(slots, "flameout", self.state == RUN && !running);
        if self.state == RUN && egt > egt_lim {
            self.exceed = self.exceed.max(egt - egt_lim);
        }
        s.put(slots, "egt_exceedance", self.exceed);
        s.put(slots, "limiter", limiter);
        s.put(slots, "mode", if alternate { 1.0 } else { 0.0 });
        s.flag(slots, "ignition_a", ign_on && (both || odd || self.attempts == 0.0));
        s.flag(slots, "ignition_b", ign_on && (both || !odd));
        s.put(slots, "attempts", self.attempts);
        s.flag(slots, "starter_overheat", self.starter_hot);
        s.flag(slots, "approach_idle_on", approach && !on_ground);
        s.flag(slots, "thrust_malfunction", self.tcm_t > 5.0);
        s.flag(slots, "shutdown", self.shutdown);
        s.put(slots, "n1_sensed", if n1r.valid { n1r.value } else { 0.0 });
        s.put(slots, "egt_sensed", egt);
        s.flag(slots, "reverser_unlocked", rev_unlocked || (self.rev > 0.0 && self.rev < 1.0));
        s.put(slots, "fuel_air", p3.map_or(0.0, |p| if p > 1e4 { self.wf / p } else { 0.0 }));
    }

    fn abort(&mut self) {
        self.state = ABORT;
        self.wf = 0.0;
        self.timer = 0.0;
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        let b = |x: bool| x as u8 as f64;
        v.extend([self.state, self.wf, self.integral, self.timer, self.egt_start, self.rev, self.exceed, self.attempts, self.starter_heat, b(self.starter_hot), self.active, self.n2_mode_ratio, self.tcm_t, b(self.shutdown), self.egt_last]);
        for p in &self.probes {
            p.state(v);
        }
    }

    pub fn restore(&mut self, it: &mut dyn Iterator<Item = f64>) {
        let mut n = || it.next().unwrap_or(0.0);
        self.state = n();
        self.wf = n();
        self.integral = n();
        self.timer = n();
        self.egt_start = n();
        self.rev = n();
        self.exceed = n();
        self.attempts = n();
        self.starter_heat = n();
        self.starter_hot = n() != 0.0;
        self.active = n().max(1.0);
        self.n2_mode_ratio = n();
        self.tcm_t = n();
        self.shutdown = n() != 0.0;
        self.egt_last = n();
        for p in &mut self.probes {
            p.restore(it);
        }
    }
}
