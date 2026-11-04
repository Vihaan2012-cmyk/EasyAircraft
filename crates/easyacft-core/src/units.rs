//! Quantities parsed from text like `"28 V"`, held in SI.

use std::fmt;

/// Exponents of mass, length, time, current and temperature.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Dim(pub [i8; 5]);

impl Dim {
    pub const NONE: Dim = Dim([0, 0, 0, 0, 0]);
    pub const MASS: Dim = Dim([1, 0, 0, 0, 0]);
    pub const LENGTH: Dim = Dim([0, 1, 0, 0, 0]);
    pub const TIME: Dim = Dim([0, 0, 1, 0, 0]);
    pub const CURRENT: Dim = Dim([0, 0, 0, 1, 0]);
    pub const TEMPERATURE: Dim = Dim([0, 0, 0, 0, 1]);
    pub const VOLTAGE: Dim = Dim([1, 2, -3, -1, 0]);
    pub const POWER: Dim = Dim([1, 2, -3, 0, 0]);
    pub const ENERGY: Dim = Dim([1, 2, -2, 0, 0]);
    pub const PRESSURE: Dim = Dim([1, -1, -2, 0, 0]);
    pub const FORCE: Dim = Dim([1, 1, -2, 0, 0]);
    pub const FREQUENCY: Dim = Dim([0, 0, -1, 0, 0]);
    pub const RESISTANCE: Dim = Dim([1, 2, -3, -2, 0]);
    pub const CHARGE: Dim = Dim([0, 0, 1, 1, 0]);
    pub const MASS_FLOW: Dim = Dim([1, 0, -1, 0, 0]);
    pub const VOLUME: Dim = Dim([0, 3, 0, 0, 0]);
    pub const DENSITY: Dim = Dim([1, -3, 0, 0, 0]);
    pub const VOLUME_FLOW: Dim = Dim([0, 3, -1, 0, 0]);
    pub const SPEED: Dim = Dim([0, 1, -1, 0, 0]);
    pub const TORQUE: Dim = Dim([1, 2, -2, 0, 0]);

    pub fn mul(self, o: Dim) -> Dim {
        let mut d = self.0;
        for (a, b) in d.iter_mut().zip(o.0) {
            *a += b;
        }
        Dim(d)
    }

    pub fn pow(self, n: i8) -> Dim {
        Dim(self.0.map(|e| e * n))
    }

    pub fn name(self) -> String {
        const KNOWN: [(Dim, &str); 20] = [
            (Dim::NONE, "a plain number"),
            (Dim::MASS, "mass"),
            (Dim::LENGTH, "length"),
            (Dim::TIME, "time"),
            (Dim::CURRENT, "current"),
            (Dim::TEMPERATURE, "temperature"),
            (Dim::VOLTAGE, "voltage"),
            (Dim::POWER, "power"),
            (Dim::ENERGY, "energy or torque"),
            (Dim::PRESSURE, "pressure"),
            (Dim::FORCE, "force"),
            (Dim::FREQUENCY, "frequency"),
            (Dim::RESISTANCE, "resistance"),
            (Dim::CHARGE, "charge"),
            (Dim::MASS_FLOW, "mass flow"),
            (Dim::VOLUME, "volume"),
            (Dim::VOLUME_FLOW, "volume flow"),
            (Dim::SPEED, "speed"),
            (Dim::TORQUE, "energy or torque"),
            (Dim([0, 2, -2, 0, 0]), "energy per mass"),
        ];
        if let Some((_, n)) = KNOWN.iter().find(|(d, _)| *d == self) {
            return (*n).to_owned();
        }
        let sym = ["kg", "m", "s", "A", "K"];
        let parts: Vec<String> = self.0.iter().zip(sym).filter(|(e, _)| **e != 0).map(|(e, s)| if *e == 1 { s.to_owned() } else { format!("{s}^{e}") }).collect();
        parts.join(" ")
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quantity {
    pub si: f64,
    pub dim: Dim,
}

impl fmt::Display for Quantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.si, self.dim.name())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum UnitError {
    NoNumber(String),
    UnknownUnit(String),
    OffsetInCompound(String),
}

impl fmt::Display for UnitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnitError::NoNumber(s) => write!(f, "`{s}` does not start with a number"),
            UnitError::UnknownUnit(u) => write!(f, "unknown unit `{u}`"),
            UnitError::OffsetInCompound(u) => write!(f, "`{u}` cannot be combined with other units; use K"),
        }
    }
}

struct Unit {
    name: &'static str,
    factor: f64,
    offset: f64,
    dim: Dim,
}

const fn u(name: &'static str, factor: f64, dim: Dim) -> Unit {
    Unit { name, factor, offset: 0.0, dim }
}

const UNITS: &[Unit] = &[
    u("kg", 1.0, Dim::MASS),
    u("g", 1e-3, Dim::MASS),
    u("t", 1e3, Dim::MASS),
    u("lb", 0.453_592_37, Dim::MASS),
    u("m", 1.0, Dim::LENGTH),
    u("mm", 1e-3, Dim::LENGTH),
    u("cm", 1e-2, Dim::LENGTH),
    u("km", 1e3, Dim::LENGTH),
    u("ft", 0.3048, Dim::LENGTH),
    u("in", 0.0254, Dim::LENGTH),
    u("nm", 1852.0, Dim::LENGTH),
    u("s", 1.0, Dim::TIME),
    u("ms", 1e-3, Dim::TIME),
    u("min", 60.0, Dim::TIME),
    u("h", 3600.0, Dim::TIME),
    u("A", 1.0, Dim::CURRENT),
    u("mA", 1e-3, Dim::CURRENT),
    u("K", 1.0, Dim::TEMPERATURE),
    Unit { name: "degC", factor: 1.0, offset: 273.15, dim: Dim::TEMPERATURE },
    Unit { name: "°C", factor: 1.0, offset: 273.15, dim: Dim::TEMPERATURE },
    Unit { name: "degF", factor: 5.0 / 9.0, offset: 459.67 * 5.0 / 9.0, dim: Dim::TEMPERATURE },
    u("V", 1.0, Dim::VOLTAGE),
    u("kV", 1e3, Dim::VOLTAGE),
    u("W", 1.0, Dim::POWER),
    u("kW", 1e3, Dim::POWER),
    u("VA", 1.0, Dim::POWER),
    u("kVA", 1e3, Dim::POWER),
    u("hp", 745.699_872, Dim::POWER),
    u("J", 1.0, Dim::ENERGY),
    u("kJ", 1e3, Dim::ENERGY),
    u("MJ", 1e6, Dim::ENERGY),
    u("Wh", 3600.0, Dim::ENERGY),
    u("Ah", 3600.0, Dim::CHARGE),
    u("C", 1.0, Dim::CHARGE),
    u("ohm", 1.0, Dim::RESISTANCE),
    u("Ω", 1.0, Dim::RESISTANCE),
    u("mohm", 1e-3, Dim::RESISTANCE),
    u("Hz", 1.0, Dim::FREQUENCY),
    u("rpm", 1.0 / 60.0, Dim::FREQUENCY),
    u("Pa", 1.0, Dim::PRESSURE),
    u("kPa", 1e3, Dim::PRESSURE),
    u("bar", 1e5, Dim::PRESSURE),
    u("mbar", 100.0, Dim::PRESSURE),
    u("hPa", 100.0, Dim::PRESSURE),
    u("psi", 6_894.757_293_168, Dim::PRESSURE),
    u("inHg", 3_386.389, Dim::PRESSURE),
    u("N", 1.0, Dim::FORCE),
    u("kN", 1e3, Dim::FORCE),
    u("lbf", 4.448_221_615_260_5, Dim::FORCE),
    u("L", 1e-3, Dim::VOLUME),
    u("gal", 3.785_411_784e-3, Dim::VOLUME),
    u("kt", 1852.0 / 3600.0, Dim::SPEED),
    u("deg", std::f64::consts::PI / 180.0, Dim::NONE),
    u("rad", 1.0, Dim::NONE),
    u("%", 0.01, Dim::NONE),
];

fn lookup(name: &str) -> Option<&'static Unit> {
    UNITS.iter().find(|u| u.name == name)
}

/// One factor of a compound unit, e.g. `s2` in `m/s2`.
fn factor(token: &str) -> Result<(f64, f64, Dim, bool), UnitError> {
    let (name, exp) = match token.find('^') {
        Some(i) => (&token[..i], token[i + 1..].parse::<i8>().map_err(|_| UnitError::UnknownUnit(token.to_owned()))?),
        None => {
            let split = token.trim_end_matches(|c: char| c.is_ascii_digit());
            if split.len() < token.len() && lookup(token).is_none() {
                (split, token[split.len()..].parse::<i8>().map_err(|_| UnitError::UnknownUnit(token.to_owned()))?)
            } else {
                (token, 1)
            }
        }
    };
    let unit = lookup(name).ok_or_else(|| UnitError::UnknownUnit(name.to_owned()))?;
    Ok((unit.factor.powi(exp as i32), unit.offset, unit.dim.pow(exp), unit.offset != 0.0))
}

pub fn parse_unit(text: &str) -> Result<(f64, f64, Dim), UnitError> {
    let text = text.trim();
    if text.is_empty() {
        return Ok((1.0, 0.0, Dim::NONE));
    }
    let mut total = (1.0, 0.0, Dim::NONE);
    let mut tokens = 0;
    let mut offset_unit = None;
    for (num, part) in text.split('/').enumerate() {
        for tok in part.split('*').map(str::trim).filter(|t| !t.is_empty()) {
            let (f, off, d, has_offset) = factor(tok)?;
            tokens += 1;
            if has_offset {
                offset_unit = Some(tok.to_owned());
            }
            if num == 0 {
                total = (total.0 * f, off, total.2.mul(d));
            } else {
                total = (total.0 / f, total.1, total.2.mul(d.pow(-1)));
            }
        }
    }
    if let Some(u) = offset_unit {
        if tokens > 1 {
            return Err(UnitError::OffsetInCompound(u));
        }
    }
    Ok(total)
}

pub fn parse_quantity(text: &str) -> Result<Quantity, UnitError> {
    let text = text.trim();
    let end = text
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || c == '.' || c == 'e' && i > 0 || c == 'E' && i > 0 || (c == '-' || c == '+') && (i == 0 || matches!(text.as_bytes()[i - 1], b'e' | b'E'))))
        .map_or(text.len(), |(i, _)| i);
    let value: f64 = text[..end].trim().parse().map_err(|_| UnitError::NoNumber(text.to_owned()))?;
    let (factor, offset, dim) = parse_unit(&text[end..])?;
    Ok(Quantity { si: value * factor + offset, dim })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> Quantity {
        parse_quantity(s).unwrap()
    }

    #[test]
    fn plain_units_convert_to_si() {
        assert_eq!(q("28 V"), Quantity { si: 28.0, dim: Dim::VOLTAGE });
        assert!((q("3000 psi").si - 20_684_271.879).abs() < 1e-2);
        assert_eq!(q("90 kVA").dim, Dim::POWER);
        assert_eq!(q("2 ms").si, 0.002);
        assert_eq!(q("44 Ah").si, 158_400.0);
        assert!((q("15 degC").si - 288.15).abs() < 1e-9);
        assert!((q("59 degF").si - 288.15).abs() < 1e-9);
        assert_eq!(q("0.5").dim, Dim::NONE);
    }

    #[test]
    fn compound_units_combine_dimensions() {
        assert_eq!(q("2.5 kg/s").dim, Dim::MASS_FLOW);
        assert!((q("3600 lb/h").si - 0.453_592_37).abs() < 1e-12);
        assert_eq!(q("9.81 m/s2").dim, Dim([0, 1, -2, 0, 0]));
        assert_eq!(q("9.81 m/s^2").dim, q("9.81 m/s2").dim);
        assert_eq!(q("100 N*m").dim, Dim::TORQUE);
        assert_eq!(q("-1.5e3 W").si, -1500.0);
    }

    #[test]
    fn mistakes_are_named() {
        assert_eq!(parse_quantity("28 volts"), Err(UnitError::UnknownUnit("volts".into())));
        assert_eq!(parse_quantity("V 28"), Err(UnitError::NoNumber("V 28".into())));
        assert!(matches!(parse_quantity("1 degC/s"), Err(UnitError::OffsetInCompound(_))));
        assert_eq!(Dim::PRESSURE.name(), "pressure");
    }
}
