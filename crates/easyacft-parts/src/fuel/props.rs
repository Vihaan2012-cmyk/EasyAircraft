//! Fuel properties by grade: density and its thermal expansion, kinematic viscosity rising steeply in the cold
//! (Walther's equation through two datasheet points), freezing and cloud points, true vapour pressure that sets
//! where it boils at altitude, how much water it can hold dissolved at each temperature, and specific heat.

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grade {
    JetA,
    JetA1,
    Jp4,
    Jp8,
    Avgas,
}

#[derive(Clone, Copy, Debug)]
pub struct Fuel {
    pub grade: Grade,
    /// Density at 15 C, kg/m3.
    pub rho15: f64,
    /// Volumetric expansion, share per kelvin.
    pub expansion: f64,
    /// Walther constants: log10(log10(nu_cSt + 0.7)) = a - b log10(T).
    a: f64,
    b: f64,
    pub freeze: f64,
    /// Vapour pressure at 38 C (Reid), Pa, and its rise per kelvin as a share.
    rvp: f64,
    rvp_k: f64,
    /// Dissolved water it holds when saturated at 15 C, kg per kg, and the share per kelvin that changes.
    water15: f64,
    water_k: f64,
    pub cp: f64,
}

/// Walther constants through (t1, nu1) and (t2, nu2), cSt.
fn walther(t1: f64, nu1: f64, t2: f64, nu2: f64) -> (f64, f64) {
    let y = |nu: f64| (nu + 0.7).log10().log10();
    let b = (y(nu1) - y(nu2)) / (t2.log10() - t1.log10());
    (y(nu1) + b * t1.log10(), b)
}

impl Fuel {
    pub fn new(grade: Grade, freeze: Option<f64>, rho15: Option<f64>) -> Self {
        // Datasheet points: viscosity at -20 C and 40 C; Reid vapour pressure; dissolved water near 70 ppm at 15 C.
        let (rho, f, nu_cold, nu_warm, rvp) = match grade {
            Grade::JetA => (804.0, 233.15, 5.5, 1.2, 1.0e3),
            Grade::JetA1 => (804.0, 226.15, 5.2, 1.2, 1.0e3),
            Grade::Jp8 => (800.0, 226.15, 5.0, 1.15, 1.0e3),
            Grade::Jp4 => (760.0, 215.15, 1.6, 0.6, 18.0e3),
            Grade::Avgas => (720.0, 215.15, 0.9, 0.45, 45.0e3),
        };
        let (a, b) = walther(253.15, nu_cold, 313.15, nu_warm);
        Fuel {
            grade,
            rho15: rho15.unwrap_or(rho),
            expansion: if grade == Grade::Avgas { 0.0012 } else { 0.00099 },
            a,
            b,
            freeze: freeze.unwrap_or(f),
            rvp,
            rvp_k: 0.035,
            water15: 70e-6,
            water_k: 0.03,
            cp: if grade == Grade::Avgas { 2100.0 } else { 2000.0 },
        }
    }

    pub fn parse(s: &str, freeze: Option<f64>, rho15: Option<f64>) -> Self {
        let g = match s {
            "jet_a1" | "jet_a-1" => Grade::JetA1,
            "jp4" | "jp-4" | "jet_b" => Grade::Jp4,
            "jp8" | "jp-8" => Grade::Jp8,
            "avgas" | "100ll" => Grade::Avgas,
            _ => Grade::JetA,
        };
        Fuel::new(g, freeze, rho15)
    }

    pub fn density(&self, t: f64) -> f64 {
        self.rho15 * (1.0 - self.expansion * (t - 288.15)).max(0.5)
    }

    /// Kinematic viscosity, m2/s; it climbs steeply toward the freezing point.
    pub fn kinematic(&self, t: f64) -> f64 {
        let t = t.max(self.freeze - 5.0).max(150.0);
        let cst = 10f64.powf(10f64.powf(self.a - self.b * t.log10())) - 0.7;
        let wax = (1.0 + 4.0 * (1.0 - (t - self.freeze) / 6.0).clamp(0.0, 1.0).powi(2)).max(1.0);
        cst.max(0.3) * 1e-6 * wax
    }

    /// True vapour pressure, Pa: what the ullage must stay above for the fuel not to boil.
    pub fn vapour(&self, t: f64) -> f64 {
        self.rvp * (self.rvp_k * (t - 311.15)).exp()
    }

    /// Water it holds dissolved when saturated, kg per kg of fuel.
    pub fn water_saturation(&self, t: f64) -> f64 {
        self.water15 * (self.water_k * (t - 288.15)).exp()
    }

    /// Cloud point: wax starts to come out of solution a few kelvin above freezing.
    pub fn cloud(&self) -> f64 {
        self.freeze + 4.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jet_fuel_thickens_and_densifies_in_the_cold() {
        let f = Fuel::new(Grade::JetA, None, None);
        let (warm, cold) = (f.kinematic(313.15), f.kinematic(253.15));
        assert!((warm - 1.2e-6).abs() < 0.05e-6 && (cold - 5.5e-6).abs() < 0.2e-6, "{warm:e} {cold:e}");
        assert!(f.kinematic(236.0) > 2.0 * cold, "waxing near the freezing point");
        assert!(f.density(233.15) > f.density(288.15));
    }

    #[test]
    fn avgas_boils_at_altitude_where_jet_fuel_does_not() {
        let (jet, avgas) = (Fuel::new(Grade::JetA, None, None), Fuel::new(Grade::Avgas, None, None));
        let ullage_fl250 = 37_600.0;
        assert!(avgas.vapour(318.15) > ullage_fl250 && jet.vapour(318.15) < ullage_fl250);
    }

    #[test]
    fn cold_fuel_holds_less_water() {
        let f = Fuel::new(Grade::JetA1, None, None);
        assert!(f.water_saturation(253.15) < 0.4 * f.water_saturation(288.15));
    }
}
