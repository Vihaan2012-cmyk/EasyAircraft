//! Hydraulic fluid properties and the flow laws every valve, orifice and seal in the actuators uses: density and
//! viscosity with temperature (Walther's equation fitted to each fluid's datasheet points), the bulk modulus with
//! pressure, temperature and entrained air, orifice flow with a smooth laminar-turbulent transition, annular seal
//! leakage, and the vapour and air-release pressures where cavitation starts.

/// Atmospheric pressure, the reference for entrained air.
pub const P_ATM: f64 = 101_325.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Phosphate ester, Skydrol LD-4 and similar: airliners.
    PhosphateEster,
    /// MIL-PRF-5606 mineral oil: older and light aircraft.
    Mineral,
    /// MIL-PRF-83282 synthetic hydrocarbon, fire resistant: military and business jets.
    Synthetic,
    /// MIL-PRF-87257, a low-temperature 83282.
    LowTemperature,
}

#[derive(Clone, Copy, Debug)]
pub struct Fluid {
    pub kind: Kind,
    /// Density at 15 C and atmospheric pressure, kg/m3.
    rho0: f64,
    /// Volumetric thermal expansion, 1/K.
    expansion: f64,
    /// Walther constants, log10(log10(nu_cSt + 0.7)) = a - b * log10(T), above and below the middle point.
    a: f64,
    b: f64,
    a_cold: f64,
    b_cold: f64,
    t_mid: f64,
    /// Isothermal tangent bulk modulus at 15 C and no pressure, Pa.
    beta0: f64,
    /// Bulk modulus rise per pascal of pressure.
    beta_p: f64,
    /// Bulk modulus fall per kelvin.
    beta_t: f64,
    /// Entrained air volume share at atmospheric pressure.
    pub air: f64,
    /// Pour point: the fluid will barely move below this, K.
    pub pour_point: f64,
    /// Specific heat, J/kg/K.
    pub cp: f64,
}

/// Walther constants from two (temperature K, viscosity cSt) points.
fn walther(t1: f64, nu1: f64, t2: f64, nu2: f64) -> (f64, f64) {
    let w = |nu: f64| (nu + 0.7).log10().log10();
    let b = (w(nu1) - w(nu2)) / (t2.log10() - t1.log10());
    (w(nu1) + b * t1.log10(), b)
}

impl Fluid {
    pub fn new(kind: Kind) -> Self {
        // Datasheet viscosities cold, at 38 or 40 C and at 99 or 100 C; densities and moduli from published data.
        let (rho0, (t0, nu0, t1, nu1, t2, nu2), beta0, pour, cp, expansion) = match kind {
            Kind::PhosphateEster => (1000.0, (219.15, 1500.0, 311.15, 10.8, 372.15, 3.93), 1.45e9, 211.0, 1750.0, 8.2e-4),
            Kind::Mineral => (860.0, (219.15, 500.0, 313.15, 13.8, 373.15, 4.95), 1.40e9, 213.0, 1880.0, 8.6e-4),
            Kind::Synthetic => (850.0, (233.15, 2000.0, 313.15, 14.1, 373.15, 3.45), 1.42e9, 218.0, 2090.0, 7.6e-4),
            Kind::LowTemperature => (845.0, (219.15, 2000.0, 313.15, 6.7, 373.15, 2.2), 1.38e9, 203.0, 2100.0, 7.9e-4),
        };
        let (a, b) = walther(t1, nu1, t2, nu2);
        let (a_cold, b_cold) = walther(t0, nu0, t1, nu1);
        Fluid { kind, rho0, expansion, a, b, a_cold, b_cold, t_mid: t1, beta0, beta_p: 6.0, beta_t: 0.0045, air: 0.005, pour_point: pour, cp }
    }

    pub fn parse(s: &str) -> Self {
        let k = match s.to_ascii_lowercase().replace(['-', ' '], "_").as_str() {
            "mineral" | "5606" | "mil_prf_5606" | "mil_h_5606" => Kind::Mineral,
            "synthetic" | "83282" | "mil_prf_83282" => Kind::Synthetic,
            "low_temperature" | "87257" | "mil_prf_87257" => Kind::LowTemperature,
            _ => Kind::PhosphateEster,
        };
        Fluid::new(k)
    }

    pub fn density(&self, t: f64, p: f64) -> f64 {
        let t = t.clamp(180.0, 500.0);
        self.rho0 * (1.0 - self.expansion * (t - 288.15)) * (1.0 + p.max(0.0) / self.beta0)
    }

    /// Kinematic viscosity, m2/s.
    pub fn kinematic(&self, t: f64) -> f64 {
        let t = t.clamp(self.pour_point - 10.0, 500.0);
        let w = if t < self.t_mid { self.a_cold - self.b_cold * t.log10() } else { self.a - self.b * t.log10() };
        let nu_cst = (10f64.powf(10f64.powf(w)) - 0.7).max(0.3);
        nu_cst * 1e-6
    }

    /// Dynamic viscosity, Pa*s.
    pub fn dynamic(&self, t: f64, p: f64) -> f64 {
        // Barus: viscosity rises with pressure, about 2e-8 per pascal for these oils.
        self.kinematic(t) * self.density(t, p) * (2.0e-8 * p.clamp(0.0, 1e8)).exp()
    }

    /// Effective tangent bulk modulus of the liquid with its entrained air at absolute pressure `p`.
    pub fn bulk(&self, t: f64, p: f64) -> f64 {
        let p = p.max(1_000.0);
        let liquid = (self.beta0 + self.beta_p * p) * (1.0 - self.beta_t * (t.clamp(180.0, 500.0) - 288.15)).max(0.3);
        let x = self.air.clamp(0.0, 0.2);
        if x <= 0.0 {
            return liquid;
        }
        // Air as a polytropic gas, compressed with the liquid; it dominates at low pressure.
        let n = 1.4;
        let ratio = (P_ATM / p).powf(1.0 / n);
        let num = (1.0 - x) + x * ratio;
        let den = (1.0 - x) / liquid + x * ratio / (n * p);
        (num / den).max(1e5)
    }

    /// Absolute pressure below which dissolved air comes out and chambers start to cavitate.
    pub fn air_release(&self) -> f64 {
        20_000.0
    }

    /// Vapour pressure: the floor no chamber can go below.
    pub fn vapour(&self) -> f64 {
        match self.kind {
            Kind::PhosphateEster => 700.0,
            _ => 300.0,
        }
    }
}

/// Flow through a sharp-edged orifice of area `area` under `dp`, smooth through zero: turbulent square-root flow
/// above the critical pressure difference, laminar and linear below it. Positive dp gives positive flow.
pub fn orifice(cd: f64, area: f64, dp: f64, rho: f64, nu: f64) -> f64 {
    if area <= 0.0 {
        return 0.0;
    }
    let dh = (4.0 * area / std::f64::consts::PI).sqrt();
    let p_cr = critical_dp(cd, dh, rho, nu);
    cd * area * (2.0 / rho).sqrt() * dp / (dp * dp + p_cr * p_cr).powf(0.25)
}

/// Pressure difference at the laminar-turbulent change, from a critical Reynolds number of 12.
pub fn critical_dp(cd: f64, dh: f64, rho: f64, nu: f64) -> f64 {
    let re_cr = 12.0;
    let v = re_cr * nu / (cd * dh.max(1e-6));
    0.5 * rho * v * v
}

/// Orifice area passing `q` at `dp`, for sizing a valve from its rated point.
pub fn area_for(cd: f64, q: f64, dp: f64, rho: f64) -> f64 {
    q.abs() / (cd * (2.0 * dp.abs().max(1.0) / rho).sqrt())
}

/// Leakage through a concentric annular gap: `d` diameter, `c` radial clearance, `l` length, `ecc` the
/// eccentricity share; laminar, so it rises as the fluid warms and thins.
pub fn annulus(d: f64, c: f64, l: f64, ecc: f64, dp: f64, mu: f64) -> f64 {
    std::f64::consts::PI * d * c.powi(3) * dp / (12.0 * mu.max(1e-6) * l.max(1e-4)) * (1.0 + 1.5 * ecc * ecc)
}

/// A laminar leakage conductance sized to pass `q` at `dp` with fluid of viscosity `mu_ref`.
#[derive(Clone, Copy, Debug)]
pub struct Leak {
    k: f64,
}

impl Leak {
    pub fn rated(q: f64, dp: f64, mu_ref: f64) -> Self {
        Leak { k: q.max(0.0) * mu_ref / dp.abs().max(1.0) }
    }
    pub fn flow(&self, dp: f64, mu: f64) -> f64 {
        self.k * dp / mu.max(1e-6)
    }
    pub fn scale(&mut self, by: f64) {
        self.k *= by.max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viscosity_matches_the_datasheet_points() {
        let f = Fluid::new(Kind::PhosphateEster);
        assert!((f.kinematic(311.15) * 1e6 - 10.8).abs() < 0.05);
        assert!((f.kinematic(372.15) * 1e6 - 3.93).abs() < 0.05);
        assert!(f.kinematic(233.15) * 1e6 > 300.0, "cold Skydrol is thick: {}", f.kinematic(233.15) * 1e6);
    }

    #[test]
    fn entrained_air_softens_the_fluid_at_low_pressure() {
        let f = Fluid::new(Kind::Mineral);
        let low = f.bulk(288.0, 2e5);
        let high = f.bulk(288.0, 2e7);
        assert!(low < 0.2 * high, "{low:e} vs {high:e}");
        assert!(high > 1.2e9);
    }

    #[test]
    fn orifice_is_odd_smooth_and_turbulent_at_high_dp() {
        let (rho, nu) = (1000.0, 10e-6);
        let a = 1e-5;
        let q = orifice(0.7, a, 1e7, rho, nu);
        let ideal = 0.7 * a * (2.0 * 1e7 / rho).sqrt();
        assert!((q / ideal - 1.0).abs() < 0.01);
        assert_eq!(orifice(0.7, a, -1e7, rho, nu), -q);
        assert!(orifice(0.7, a, 0.0, rho, nu) == 0.0);
        let tiny = orifice(0.7, a, 1.0, rho, nu);
        assert!(tiny.is_finite() && tiny > 0.0);
    }
}
