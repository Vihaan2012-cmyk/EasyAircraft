//! Moist-air thermodynamics and the small control pieces every air-conditioning part shares.

pub const R: f64 = 287.05;
pub const R_VAPOUR: f64 = 461.5;
pub const CP: f64 = 1005.0;
pub const CP_VAPOUR: f64 = 1860.0;
pub const CV: f64 = CP - R;
pub const GAMMA: f64 = 1.4;
pub const LATENT: f64 = 2.501e6;
pub const LATENT_ICE: f64 = 2.834e6;
pub const P_SL: f64 = 101_325.0;
pub const T_ZERO: f64 = 273.15;
pub const PSI: f64 = 6894.757;

/// A stream of moist air: mass flow of dry air, temperature, pressure and humidity ratio (kg water per kg dry air).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stream {
    pub flow: f64,
    pub t: f64,
    pub p: f64,
    pub w: f64,
}

impl Stream {
    pub fn new(flow: f64, t: f64, p: f64, w: f64) -> Self {
        Stream { flow: flow.max(0.0), t, p, w: w.max(0.0) }
    }

    pub fn none(t: f64, p: f64) -> Self {
        Stream { flow: 0.0, t, p, w: 0.0 }
    }

    /// Specific enthalpy per kg of dry air, J/kg, with liquid water carried as free water at the same temperature.
    pub fn enthalpy(&self) -> f64 {
        enthalpy(self.t, self.w)
    }

    pub fn relative_humidity(&self) -> f64 {
        relative_humidity(self.w, self.t, self.p)
    }

    pub fn dew_point(&self) -> f64 {
        dew_point(self.w, self.p)
    }

    pub fn with_flow(mut self, flow: f64) -> Self {
        self.flow = flow.max(0.0);
        self
    }

    pub fn with_t(mut self, t: f64) -> Self {
        self.t = t;
        self
    }

    pub fn density(&self) -> f64 {
        self.p.max(1.0) / (R * self.t.max(100.0))
    }
}

pub fn enthalpy(t: f64, w: f64) -> f64 {
    let c = t - T_ZERO;
    CP * c + w * (LATENT + CP_VAPOUR * c)
}

/// Temperature of moist air with humidity `w` holding specific enthalpy `h`.
pub fn temperature_of(h: f64, w: f64) -> f64 {
    T_ZERO + (h - w * LATENT) / (CP + w * CP_VAPOUR)
}

/// Adiabatic mixing by enthalpy and moisture; pressure is the flow-weighted mean.
pub fn mix(streams: &[Stream]) -> Stream {
    let total: f64 = streams.iter().map(|s| s.flow).sum();
    if total <= 1e-9 {
        let s = streams.first().copied().unwrap_or(Stream::none(288.15, P_SL));
        return Stream { flow: 0.0, ..s };
    }
    let h = streams.iter().map(|s| s.flow * s.enthalpy()).sum::<f64>() / total;
    let w = streams.iter().map(|s| s.flow * s.w).sum::<f64>() / total;
    let p = streams.iter().map(|s| s.flow * s.p).sum::<f64>() / total;
    Stream { flow: total, t: temperature_of(h, w), p, w }
}

/// Saturation vapour pressure over water, or ice below freezing (Magnus, Alduchov-Eskridge constants).
pub fn p_sat(t: f64) -> f64 {
    let c = t - T_ZERO;
    if c >= 0.0 {
        610.94 * (17.625 * c / (c + 243.04)).exp()
    } else {
        611.21 * (22.587 * c / (c + 273.86)).exp()
    }
}

pub fn w_sat(t: f64, p: f64) -> f64 {
    let ps = p_sat(t).min(0.99 * p.max(1.0));
    0.622 * ps / (p.max(1.0) - ps)
}

pub fn relative_humidity(w: f64, t: f64, p: f64) -> f64 {
    let pv = p.max(1.0) * w / (0.622 + w);
    (pv / p_sat(t)).clamp(0.0, 1.5)
}

pub fn humidity_ratio(rh: f64, t: f64, p: f64) -> f64 {
    let pv = (rh.clamp(0.0, 1.0) * p_sat(t)).min(0.99 * p.max(1.0));
    0.622 * pv / (p.max(1.0) - pv)
}

/// Dew point (frost point below freezing) of air with humidity ratio `w` at `p`.
pub fn dew_point(w: f64, p: f64) -> f64 {
    let pv = (p.max(1.0) * w / (0.622 + w)).max(1e-3);
    let x = (pv / 610.94).ln();
    let c = 243.04 * x / (17.625 - x);
    if c >= 0.0 {
        return T_ZERO + c;
    }
    let y = (pv / 611.21).ln();
    T_ZERO + 273.86 * y / (22.587 - y)
}

/// Condense what a stream cannot hold at its temperature, releasing latent heat; returns the stream and the
/// water mass flow condensed (liquid, or ice below freezing).
pub fn condense(s: Stream) -> (Stream, f64) {
    if s.w <= w_sat(s.t, s.p) {
        return (s, 0.0);
    }
    // Solve for the humidity that is saturated at the temperature its own latent heat leaves the air at.
    let latent = if s.t < T_ZERO { LATENT_ICE } else { LATENT };
    let warmed = |w: f64| s.t + (s.w - w) * latent / (CP + w * CP_VAPOUR);
    let (mut lo, mut hi) = (0.0, s.w);
    for _ in 0..50 {
        let mid = 0.5 * (lo + hi);
        if w_sat(warmed(mid), s.p) > mid { lo = mid } else { hi = mid }
    }
    let w = 0.5 * (lo + hi);
    let out = Stream { t: warmed(w), w, ..s };
    (out, (s.w - w).max(0.0) * s.flow)
}

/// Compressor outlet temperature for a pressure ratio and isentropic efficiency.
pub fn compress(t_in: f64, ratio: f64, efficiency: f64) -> f64 {
    let ideal = t_in * ratio.max(1.0).powf((GAMMA - 1.0) / GAMMA);
    t_in + (ideal - t_in) / efficiency.clamp(0.05, 1.0)
}

/// Turbine outlet temperature expanding through a pressure ratio (inlet over outlet) at an isentropic efficiency.
pub fn expand(t_in: f64, ratio: f64, efficiency: f64) -> f64 {
    let ideal = t_in / ratio.max(1.0).powf((GAMMA - 1.0) / GAMMA);
    t_in - (t_in - ideal) * efficiency.clamp(0.0, 1.0)
}

/// Cross-flow heat exchanger, both fluids unmixed, by effectiveness-NTU. Returns hot outlet, cold outlet, heat.
pub fn exchanger(t_hot: f64, m_hot: f64, t_cold: f64, m_cold: f64, ua: f64) -> (f64, f64, f64) {
    let (c_hot, c_cold) = (m_hot.max(0.0) * CP, m_cold.max(0.0) * CP);
    let (c_min, c_max) = (c_hot.min(c_cold), c_hot.max(c_cold));
    if c_min <= 1e-9 || ua <= 0.0 {
        return (t_hot, t_cold, 0.0);
    }
    let ntu = ua / c_min;
    let cr = c_min / c_max;
    let eff = (1.0 - ((ntu.powf(0.22) / cr) * ((-cr * ntu.powf(0.78)).exp() - 1.0)).exp()).clamp(0.0, 1.0);
    let q = eff * c_min * (t_hot - t_cold);
    (t_hot - q / c_hot.max(1e-9), t_cold + q / c_cold.max(1e-9), q)
}

/// Total temperature at a probe or on the skin with recovery factor `r`.
pub fn recovery_temperature(t_static: f64, speed: f64, r: f64) -> f64 {
    t_static + r * speed * speed / (2.0 * CP)
}

/// Ram (total) pressure for a free stream at Mach `m`, recovered with efficiency `eta`.
pub fn ram_pressure(p_static: f64, mach: f64, eta: f64) -> f64 {
    let ideal = p_static * (1.0 + 0.2 * mach * mach).powf(3.5);
    p_static + (ideal - p_static) * eta.clamp(0.0, 1.0)
}

/// Mass flow through an orifice, compressible and choked (re-exported from the cabin model).
pub fn orifice(cda: f64, p_up: f64, t_up: f64, p_down: f64) -> f64 {
    crate::cabin::orifice(cda, p_up, t_up, p_down)
}

/// Signed flow from side a to side b through an orifice.
pub fn orifice_signed(cda: f64, pa: f64, ta: f64, pb: f64, tb: f64) -> f64 {
    if pa >= pb { orifice(cda, pa, ta, pb) } else { -orifice(cda, pb, tb, pa) }
}

pub fn lag(x: f64, target: f64, tau: f64, dt: f64) -> f64 {
    if tau <= 0.0 { target } else { x + (target - x) * (1.0 - (-dt / tau).exp()) }
}

pub fn isa_altitude(p: f64) -> f64 {
    crate::cabin::isa_altitude(p)
}

pub fn isa_pressure(h: f64) -> f64 {
    crate::cabin::isa_pressure(h)
}

/// A motor- or pneumatically-driven valve or door: position 0 to 1 at a travel rate, that can jam.
#[derive(Clone, Copy, Debug)]
pub struct Actuator {
    pub position: f64,
    pub travel_time: f64,
    pub jammed: bool,
}

impl Actuator {
    pub fn new(position: f64, travel_time: f64) -> Self {
        Actuator { position: position.clamp(0.0, 1.0), travel_time: travel_time.max(0.0), jammed: false }
    }

    /// Drive toward `target` at the travel rate scaled by `speed` (0 for an unpowered motor).
    pub fn drive(&mut self, target: f64, speed: f64, dt: f64) -> f64 {
        if self.jammed {
            return self.position;
        }
        let target = target.clamp(0.0, 1.0);
        self.position = if self.travel_time <= 0.0 {
            if speed > 0.0 { target } else { self.position }
        } else {
            let step = speed.max(0.0) * dt / self.travel_time;
            self.position + (target - self.position).clamp(-step, step)
        };
        self.position
    }

    pub fn state(&self) -> [f64; 2] {
        [self.position, self.jammed as u8 as f64]
    }

    pub fn restore(&mut self, s: [f64; 2]) {
        self.position = s[0];
        self.jammed = s[1] != 0.0;
    }
}

/// A proportional-integral controller with anti-windup by clamping the integral to the output range.
#[derive(Clone, Copy, Debug)]
pub struct Pi {
    pub kp: f64,
    pub ki: f64,
    pub min: f64,
    pub max: f64,
    pub integral: f64,
}

impl Pi {
    pub fn new(kp: f64, ki: f64, min: f64, max: f64) -> Self {
        Pi { kp, ki, min, max, integral: 0.0 }
    }

    pub fn starting_at(mut self, v: f64) -> Self {
        self.integral = v.clamp(self.min, self.max);
        self
    }

    pub fn step(&mut self, error: f64, dt: f64) -> f64 {
        self.integral = (self.integral + self.ki * error * dt).clamp(self.min, self.max);
        (self.kp * error + self.integral).clamp(self.min, self.max)
    }

    pub fn reset(&mut self, to: f64) {
        self.integral = to.clamp(self.min, self.max);
    }
}

/// A first-order sensor with lag, bias and failure to a fixed reading.
#[derive(Clone, Copy, Debug)]
pub struct Sensor {
    pub value: f64,
    pub tau: f64,
    pub bias: f64,
}

impl Sensor {
    pub fn new(value: f64, tau: f64) -> Self {
        Sensor { value, tau, bias: 0.0 }
    }

    pub fn read(&mut self, truth: f64, failed: bool, dt: f64) -> f64 {
        if !failed {
            self.value = lag(self.value, truth, self.tau, dt);
        }
        self.value + self.bias
    }
}

/// A timer that runs while its condition holds and reports once it has held for `delay`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Confirm {
    pub elapsed: f64,
}

impl Confirm {
    pub fn hold(&mut self, cond: bool, delay: f64, dt: f64) -> bool {
        self.elapsed = if cond { self.elapsed + dt } else { 0.0 };
        cond && self.elapsed >= delay
    }
}

/// A latch set by `set` and cleared by `reset`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Latch {
    pub on: bool,
}

impl Latch {
    pub fn update(&mut self, set: bool, reset: bool) -> bool {
        if reset {
            self.on = false;
        } else if set {
            self.on = true;
        }
        self.on
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humidity_and_dew_point_agree() {
        let w = humidity_ratio(0.5, 298.15, P_SL);
        assert!((w - 0.0099).abs() < 0.0005, "{w}");
        let dp = dew_point(w, P_SL);
        assert!((dp - 287.0).abs() < 0.6, "{dp}");
        assert!((relative_humidity(w, 298.15, P_SL) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn condensing_warms_the_air_and_leaves_it_saturated() {
        let s = Stream::new(1.0, 280.0, 2.0 * P_SL, 0.012);
        let (out, water) = condense(s);
        assert!(water > 0.0 && out.t > s.t);
        assert!((out.relative_humidity() - 1.0).abs() < 0.05);
    }

    #[test]
    fn mixing_conserves_enthalpy_and_water() {
        let a = Stream::new(1.0, 300.0, P_SL, 0.01);
        let b = Stream::new(3.0, 280.0, P_SL, 0.002);
        let m = mix(&[a, b]);
        assert!((m.flow - 4.0).abs() < 1e-12);
        assert!((m.w - 0.004).abs() < 1e-12);
        let h = (a.enthalpy() + 3.0 * b.enthalpy()) / 4.0;
        assert!((m.enthalpy() - h).abs() < 1e-6);
    }

    #[test]
    fn a_compressor_and_turbine_pair_cools_with_losses() {
        let hot = compress(300.0, 2.5, 0.8);
        assert!(hot > 300.0 * 2.5f64.powf(0.2857));
        let cold = expand(hot, 2.5, 0.8);
        assert!(cold > 300.0 && cold < hot);
        let (h, c, q) = exchanger(450.0, 1.0, 300.0, 2.0, 3000.0);
        assert!(h < 450.0 && c > 300.0 && (q - (450.0 - h) * CP).abs() < 1e-6);
    }
}
