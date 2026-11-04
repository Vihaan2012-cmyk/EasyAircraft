//! Real-time cabin airflow: an incompressible solver on a staggered (MAC) voxel grid shaped to the fuselage
//! section. Velocities live on cell faces; semi-Lagrangian advection, eddy diffusion, Boussinesq buoyancy and an
//! SOR pressure projection with the supply diffusers and exhaust grilles as mass sources. Temperature, humidity and
//! CO2 ride on the flow, seated passengers are heat, moisture and CO2 sources, and the fuselage wall loses heat to
//! the skin. Coarse by design: it resolves stratification, plumes, draughts and dead spots, not turbulence.

use super::thermo::{CP, LATENT};

const G: f64 = 9.81;
/// Turbulent (eddy) viscosity and diffusivity of cabin air, m²/s.
const EDDY: f64 = 0.012;
const PROJECTION_ITERATIONS: usize = 60;
const SOR: f64 = 1.7;
/// Sensible and latent heat and CO2 of a seated, resting adult.
const PAX_SENSIBLE: f64 = 75.0;
const PAX_LATENT: f64 = 45.0;
const PAX_CO2: f64 = 5.2e-6;
/// Seated head and ankle heights above the floor, m.
pub const HEAD: f64 = 1.1;
pub const ANKLE: f64 = 0.1;
/// Seat row pitch and aisle width, m.
const SEAT_PITCH: f64 = 0.8;
const AISLE: f64 = 0.5;
/// Fastest a diffuser jet leaves its slot, m/s.
const JET_MAX: f64 = 2.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cell {
    Solid,
    Air,
    /// Air next to the fuselage skin.
    Wall,
    Supply,
    Exhaust,
}

/// The fuselage cross-section a zone sits in, centred on the fuselage axis: y across, z up, metres.
#[derive(Clone, Debug)]
pub enum Section {
    Circle { radius: f64 },
    Ellipse { width: f64, height: f64 },
    Rectangle { width: f64, height: f64 },
    /// Corner points in order around the section.
    Polygon(Vec<(f64, f64)>),
}

impl Section {
    pub fn contains(&self, y: f64, z: f64) -> bool {
        match self {
            Section::Circle { radius } => y * y + z * z < radius * radius,
            Section::Ellipse { width, height } => {
                let (a, b) = (0.5 * width, 0.5 * height);
                (y / a).powi(2) + (z / b).powi(2) < 1.0
            }
            Section::Rectangle { width, height } => y.abs() < 0.5 * width && z.abs() < 0.5 * height,
            Section::Polygon(pts) => {
                let mut inside = false;
                let n = pts.len();
                for i in 0..n {
                    let (a, b) = (pts[i], pts[(i + n - 1) % n]);
                    if (a.1 > z) != (b.1 > z) && y < (b.0 - a.0) * (z - a.1) / (b.1 - a.1) + a.0 {
                        inside = !inside;
                    }
                }
                inside
            }
        }
    }

    /// Half the widest extent and the top and bottom of the section.
    pub fn extent(&self) -> (f64, f64, f64) {
        match self {
            Section::Circle { radius } => (*radius, *radius, -radius),
            Section::Ellipse { width, height } | Section::Rectangle { width, height } => (0.5 * width, 0.5 * height, -0.5 * height),
            Section::Polygon(pts) => {
                let half = pts.iter().map(|p| p.0.abs()).fold(0.0, f64::max);
                let top = pts.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
                let bottom = pts.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
                (half, top, bottom)
            }
        }
    }

    /// Parse "circle", "ellipse", "rectangle" or "polygon" with its sizes; polygon points as "y,z y,z ...".
    pub fn parse(kind: &str, radius: f64, width: f64, height: f64, points: &str) -> Section {
        match kind {
            "ellipse" | "oval" => Section::Ellipse { width: width.max(0.5), height: height.max(0.5) },
            "rectangle" | "box" => Section::Rectangle { width: width.max(0.5), height: height.max(0.5) },
            "polygon" => {
                let pts: Vec<(f64, f64)> = points
                    .split(|c: char| c == ';' || c.is_whitespace())
                    .filter_map(|pair| {
                        let mut it = pair.split(',').map(|x| x.trim().parse::<f64>());
                        match (it.next(), it.next()) {
                            (Some(Ok(y)), Some(Ok(z))) => Some((y, z)),
                            _ => None,
                        }
                    })
                    .collect();
                if pts.len() >= 3 { Section::Polygon(pts) } else { Section::Circle { radius: radius.max(0.5) } }
            }
            _ => Section::Circle { radius: radius.max(0.5) },
        }
    }
}

/// Geometry of one zone's grid.
#[derive(Clone, Debug)]
pub struct Shape {
    pub section: Section,
    pub length: f64,
    pub floor: f64,
    pub ceiling: f64,
    pub cell: f64,
    pub seats_abreast: f64,
    pub bins: bool,
}

/// What the zone feeds in for one solver step.
#[derive(Clone, Copy, Debug)]
pub struct Inputs {
    pub supply_flow: f64,
    pub supply_t: f64,
    pub supply_q: f64,
    pub supply_co2: f64,
    /// Smoke produced, kg/s, and where along, across and up the zone, each 0 to 1.
    pub smoke_rate: f64,
    pub smoke_at: (f64, f64, f64),
    /// Air taken out by extract fans, kg/s.
    pub extract: f64,
    pub occupants: f64,
    pub heat: f64,
    pub skin_t: f64,
    pub skin_ua: f64,
    pub density: f64,
    pub furnishing_t: f64,
    pub furnishing_ua: f64,
}

/// What the grid reports back.
#[derive(Clone, Copy, Debug, Default)]
pub struct Outputs {
    pub mean_t: f64,
    pub head_t: f64,
    pub ankle_t: f64,
    pub ceiling_t: f64,
    pub sensor_t: f64,
    pub exhaust_t: f64,
    pub exhaust_q: f64,
    pub exhaust_co2: f64,
    pub min_t: f64,
    pub max_t: f64,
    pub draught: f64,
    pub mean_speed: f64,
    pub mean_q: f64,
    pub mean_co2: f64,
    pub seat_co2_max: f64,
    pub smoke_detector: f64,
    pub smoke_max: f64,
    pub smoke_mean: f64,
    pub wall_loss: f64,
    pub furnishing_gain: f64,
}

/// A staggered field's dimensions: fastest index first.
#[derive(Clone, Copy, Debug)]
struct Dims {
    a: usize,
    b: usize,
    c: usize,
}

impl Dims {
    #[inline]
    fn at(&self, i: usize, j: usize, k: usize) -> usize {
        (k * self.b + j) * self.a + i
    }

    fn len(&self) -> usize {
        self.a * self.b * self.c
    }
}

pub struct Grid {
    pub nx: usize,
    pub ny: usize,
    pub nz: usize,
    pub h: f64,
    pub kind: Vec<Cell>,
    wall_faces: Vec<u8>,
    seat: Vec<bool>,
    du: Dims,
    dv: Dims,
    dw: Dims,
    /// Face velocities, m/s, and whether each face is open (fluid on both sides).
    pub u: Vec<f64>,
    pub v: Vec<f64>,
    pub w: Vec<f64>,
    ou: Vec<bool>,
    ov: Vec<bool>,
    ow: Vec<bool>,
    pub t: Vec<f64>,
    pub q: Vec<f64>,
    pub co2: Vec<f64>,
    /// Smoke, mg/m³.
    pub smoke: Vec<f64>,
    pressure: Vec<f64>,
    div: Vec<f64>,
    open_faces: Vec<u8>,
    scratch: Vec<f64>,
    supply_cells: usize,
    exhaust_cells: usize,
    seat_cells: usize,
    sensor: usize,
    /// Ceiling smoke detector.
    detector: usize,
}

fn trilinear(f: &[f64], d: Dims, fx: f64, fy: f64, fz: f64) -> f64 {
    let fx = fx.clamp(0.0, (d.a - 1) as f64);
    let fy = fy.clamp(0.0, (d.b - 1) as f64);
    let fz = fz.clamp(0.0, (d.c - 1) as f64);
    let (i0, j0, k0) = (fx.floor() as usize, fy.floor() as usize, fz.floor() as usize);
    let (i1, j1, k1) = ((i0 + 1).min(d.a - 1), (j0 + 1).min(d.b - 1), (k0 + 1).min(d.c - 1));
    let (sx, sy, sz) = (fx - i0 as f64, fy - j0 as f64, fz - k0 as f64);
    let lerp = |a: f64, b: f64, s: f64| a + (b - a) * s;
    let c00 = lerp(f[d.at(i0, j0, k0)], f[d.at(i1, j0, k0)], sx);
    let c10 = lerp(f[d.at(i0, j1, k0)], f[d.at(i1, j1, k0)], sx);
    let c01 = lerp(f[d.at(i0, j0, k1)], f[d.at(i1, j0, k1)], sx);
    let c11 = lerp(f[d.at(i0, j1, k1)], f[d.at(i1, j1, k1)], sx);
    lerp(lerp(c00, c10, sy), lerp(c01, c11, sy), sz)
}

impl Grid {
    pub fn new(shape: Shape, t0: f64, q0: f64, co2_0: f64) -> Self {
        let h = shape.cell.clamp(0.15, 2.0);
        let nx = ((shape.length / h).round() as usize).clamp(2, 200);
        let (half, sec_top, sec_bottom) = shape.section.extent();
        let ny = ((2.0 * half / h).round() as usize).clamp(3, 80);
        let z_bottom = shape.floor.max(sec_bottom);
        let z_top = shape.ceiling.min(sec_top).max(z_bottom + 2.0 * h);
        let nz = (((z_top - z_bottom) / h).round() as usize).clamp(2, 40);
        let n = nx * ny * nz;
        let mut kind = vec![Cell::Solid; n];
        let y_of = |j: usize| -half + (j as f64 + 0.5) * h;
        let z_of = |k: usize| z_bottom + (k as f64 + 0.5) * h;
        let inside = |j: usize, k: usize| {
            let (y, z) = (y_of(j), z_of(k));
            shape.section.contains(y, z) && z > z_bottom && z < z_top
        };
        let idx = |i: usize, j: usize, k: usize| (k * ny + j) * nx + i;
        for k in 0..nz {
            for j in 0..ny {
                if inside(j, k) {
                    for i in 0..nx {
                        kind[idx(i, j, k)] = Cell::Air;
                    }
                }
            }
        }
        // Overhead bins: solid blocks high and outboard on each side.
        let bin_z = z_top - 0.45 * (z_top - z_bottom);
        for k in 0..nz {
            for j in 0..ny {
                let (y, z) = (y_of(j), z_of(k));
                if shape.bins && z > bin_z && y.abs() > 0.55 * half && z < z_top - 0.5 * h {
                    for i in 0..nx {
                        kind[idx(i, j, k)] = Cell::Solid;
                    }
                }
            }
        }
        // Wall cells touch the outside; count their skin faces for heat loss (the floor is not skin).
        let mut wall_faces = vec![0u8; n];
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    let c = idx(i, j, k);
                    if kind[c] == Cell::Solid {
                        continue;
                    }
                    let mut faces = 0u8;
                    for (dj, dk) in [(-1i64, 0i64), (1, 0), (0, -1), (0, 1)] {
                        let (jj, kk) = (j as i64 + dj, k as i64 + dk);
                        let off = jj < 0 || kk < 0 || jj >= ny as i64 || kk >= nz as i64;
                        let outside = off || !inside(jj as usize, kk as usize);
                        if outside && !(dk == -1) {
                            faces += 1;
                        }
                    }
                    if faces > 0 {
                        kind[c] = Cell::Wall;
                        wall_faces[c] = faces;
                    }
                }
            }
        }
        // Supply diffusers along the ceiling just inboard of the bins; exhaust grilles low on the sidewalls.
        let (mut sup, mut exh) = (0, 0);
        for j in 0..ny {
            let y = y_of(j);
            let k_sup = (0..nz).rev().find(|&k| kind[idx(0, j, k)] != Cell::Solid && z_of(k) < z_top - 0.2);
            if let Some(k) = k_sup {
                if y.abs() > 0.3 * half && y.abs() < 0.6 * half {
                    for i in 0..nx {
                        kind[idx(i, j, k)] = Cell::Supply;
                        sup += 1;
                    }
                }
            }
        }
        for k in 0..nz.min(2) {
            for j in 0..ny {
                if matches!(kind[idx(0, j, k)], Cell::Wall) && y_of(j).abs() > 0.6 * half {
                    for i in 0..nx {
                        kind[idx(i, j, k)] = Cell::Exhaust;
                        exh += 1;
                    }
                }
            }
        }
        if exh == 0 {
            for j in [0, ny - 1] {
                if let Some(k) = (0..nz).find(|&k| kind[idx(0, j, k)] != Cell::Solid) {
                    for i in 0..nx {
                        kind[idx(i, j, k)] = Cell::Exhaust;
                        exh += 1;
                    }
                }
            }
        }
        // Passengers sit in rows at seat pitch, either side of a clear aisle, from seat to head height.
        let mut seat = vec![false; n];
        let mut seats = 0;
        let usable = (shape.seats_abreast * 0.55).max(0.5);
        let pitch = ((SEAT_PITCH / h).round() as usize).max(1);
        for k in 0..nz {
            let z = z_of(k) - z_bottom;
            if !(0.3..=HEAD + 0.2).contains(&z) {
                continue;
            }
            for j in 0..ny {
                let y = y_of(j).abs();
                if y > usable.min(0.85 * half) || y < AISLE * 0.5 {
                    continue;
                }
                for i in (0..nx).filter(|i| i % pitch == 0) {
                    let c = idx(i, j, k);
                    if matches!(kind[c], Cell::Air | Cell::Wall) {
                        seat[c] = true;
                        seats += 1;
                    }
                }
            }
        }
        if seats == 0 {
            for c in 0..n {
                if matches!(kind[c], Cell::Air) {
                    seat[c] = true;
                    seats += 1;
                }
            }
        }
        // The zone sensor sits in the aisle lining at seated head height.
        let sensor_j = ny / 2;
        let head_k = ((HEAD / h - 0.5).round().max(0.0) as usize).min(nz - 1);
        let sensor_k = (0..=head_k).rev().find(|&k| kind[idx(nx / 2, sensor_j, k)] != Cell::Solid).unwrap_or(nz / 2);
        let du = Dims { a: nx + 1, b: ny, c: nz };
        let dv = Dims { a: nx, b: ny + 1, c: nz };
        let dw = Dims { a: nx, b: ny, c: nz + 1 };
        let fluid = |i: i64, j: i64, k: i64| i >= 0 && j >= 0 && k >= 0 && (i as usize) < nx && (j as usize) < ny && (k as usize) < nz && kind[idx(i as usize, j as usize, k as usize)] != Cell::Solid;
        let mut ou = vec![false; du.len()];
        let mut ov = vec![false; dv.len()];
        let mut ow = vec![false; dw.len()];
        for k in 0..nz as i64 {
            for j in 0..ny as i64 {
                for i in 0..=nx as i64 {
                    ou[du.at(i as usize, j as usize, k as usize)] = fluid(i - 1, j, k) && fluid(i, j, k);
                }
            }
        }
        for k in 0..nz as i64 {
            for j in 0..=ny as i64 {
                for i in 0..nx as i64 {
                    ov[dv.at(i as usize, j as usize, k as usize)] = fluid(i, j - 1, k) && fluid(i, j, k);
                }
            }
        }
        for k in 0..=nz as i64 {
            for j in 0..ny as i64 {
                for i in 0..nx as i64 {
                    ow[dw.at(i as usize, j as usize, k as usize)] = fluid(i, j, k - 1) && fluid(i, j, k);
                }
            }
        }
        let mut open_faces = vec![0u8; n];
        for k in 0..nz {
            for j in 0..ny {
                for i in 0..nx {
                    let c = idx(i, j, k);
                    open_faces[c] = [ou[du.at(i, j, k)], ou[du.at(i + 1, j, k)], ov[dv.at(i, j, k)], ov[dv.at(i, j + 1, k)], ow[dw.at(i, j, k)], ow[dw.at(i, j, k + 1)]].iter().filter(|o| **o).count() as u8;
                }
            }
        }
        let big = du.len().max(dv.len()).max(dw.len()).max(n);
        let detector = idx(nx / 2, sensor_j, (0..nz).rev().find(|&k| kind[idx(nx / 2, sensor_j, k)] != Cell::Solid).unwrap_or(nz - 1));
        Grid {
            nx,
            ny,
            nz,
            h,
            kind,
            wall_faces,
            seat,
            du,
            dv,
            dw,
            u: vec![0.0; du.len()],
            v: vec![0.0; dv.len()],
            w: vec![0.0; dw.len()],
            ou,
            ov,
            ow,
            t: vec![t0; n],
            q: vec![q0; n],
            co2: vec![co2_0; n],
            smoke: vec![0.0; n],
            pressure: vec![0.0; n],
            div: vec![0.0; n],
            open_faces,
            scratch: vec![0.0; big],
            supply_cells: sup.max(1),
            exhaust_cells: exh.max(1),
            seat_cells: seats.max(1),
            sensor: idx(nx / 2, sensor_j, sensor_k),
            detector,
        }
    }

    #[inline]
    pub fn idx(&self, i: usize, j: usize, k: usize) -> usize {
        (k * self.ny + j) * self.nx + i
    }

    pub fn fluid(&self, c: usize) -> bool {
        self.kind[c] != Cell::Solid
    }

    pub fn cells(&self) -> usize {
        self.kind.iter().filter(|k| **k != Cell::Solid).count()
    }

    /// Velocity at a point in grid coordinates (metres from the grid's low corner).
    fn velocity(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        let h = self.h;
        (
            trilinear(&self.u, self.du, x / h, y / h - 0.5, z / h - 0.5),
            trilinear(&self.v, self.dv, x / h - 0.5, y / h, z / h - 0.5),
            trilinear(&self.w, self.dw, x / h - 0.5, y / h - 0.5, z / h),
        )
    }

    /// A cell-centred scalar at a point, averaging only over fluid cells.
    fn scalar(&self, f: &[f64], x: f64, y: f64, z: f64) -> f64 {
        let fx = (x / self.h - 0.5).clamp(0.0, (self.nx - 1) as f64);
        let fy = (y / self.h - 0.5).clamp(0.0, (self.ny - 1) as f64);
        let fz = (z / self.h - 0.5).clamp(0.0, (self.nz - 1) as f64);
        let (i0, j0, k0) = (fx.floor() as usize, fy.floor() as usize, fz.floor() as usize);
        let (i1, j1, k1) = ((i0 + 1).min(self.nx - 1), (j0 + 1).min(self.ny - 1), (k0 + 1).min(self.nz - 1));
        let (sx, sy, sz) = (fx - i0 as f64, fy - j0 as f64, fz - k0 as f64);
        let (mut acc, mut wsum) = (0.0, 0.0);
        for (i, wi) in [(i0, 1.0 - sx), (i1, sx)] {
            for (j, wj) in [(j0, 1.0 - sy), (j1, sy)] {
                for (k, wk) in [(k0, 1.0 - sz), (k1, sz)] {
                    let c = self.idx(i, j, k);
                    let wgt = wi * wj * wk;
                    if wgt > 0.0 && self.fluid(c) {
                        acc += wgt * f[c];
                        wsum += wgt;
                    }
                }
            }
        }
        if wsum > 1e-9 { acc / wsum } else { f[self.idx(i0, j0, k0)] }
    }

    /// Cell-centred velocity from the faces around a cell.
    pub fn cell_velocity(&self, i: usize, j: usize, k: usize) -> (f64, f64, f64) {
        (
            0.5 * (self.u[self.du.at(i, j, k)] + self.u[self.du.at(i + 1, j, k)]),
            0.5 * (self.v[self.dv.at(i, j, k)] + self.v[self.dv.at(i, j + 1, k)]),
            0.5 * (self.w[self.dw.at(i, j, k)] + self.w[self.dw.at(i, j, k + 1)]),
        )
    }

    /// Semi-Lagrangian advection of each face velocity component.
    fn advect_velocity(&mut self, dt: f64) {
        let h = self.h;
        let mut nu = std::mem::take(&mut self.scratch);
        nu.resize(self.du.len().max(self.dv.len()).max(self.dw.len()).max(self.kind.len()), 0.0);
        for comp in 0..3 {
            let (d, open) = match comp {
                0 => (self.du, &self.ou),
                1 => (self.dv, &self.ov),
                _ => (self.dw, &self.ow),
            };
            for k in 0..d.c {
                for j in 0..d.b {
                    for i in 0..d.a {
                        let f = d.at(i, j, k);
                        if !open[f] {
                            nu[f] = 0.0;
                            continue;
                        }
                        let (x, y, z) = match comp {
                            0 => (i as f64 * h, (j as f64 + 0.5) * h, (k as f64 + 0.5) * h),
                            1 => ((i as f64 + 0.5) * h, j as f64 * h, (k as f64 + 0.5) * h),
                            _ => ((i as f64 + 0.5) * h, (j as f64 + 0.5) * h, k as f64 * h),
                        };
                        let (vu, vv, vw) = self.velocity(x, y, z);
                        let (bx, by, bz) = (x - vu * dt, y - vv * dt, z - vw * dt);
                        nu[f] = match comp {
                            0 => trilinear(&self.u, self.du, bx / h, by / h - 0.5, bz / h - 0.5),
                            1 => trilinear(&self.v, self.dv, bx / h - 0.5, by / h, bz / h - 0.5),
                            _ => trilinear(&self.w, self.dw, bx / h - 0.5, by / h - 0.5, bz / h),
                        };
                    }
                }
            }
            let target = match comp {
                0 => &mut self.u,
                1 => &mut self.v,
                _ => &mut self.w,
            };
            target.copy_from_slice(&nu[..d.len()]);
        }
        self.scratch = nu;
    }

    /// Explicit eddy viscosity on each face field, no-slip at closed faces, sub-stepped for stability.
    fn diffuse_velocity(&mut self, dt: f64) {
        let limit = self.h * self.h / (6.5 * EDDY);
        let steps = (dt / limit).ceil().max(1.0) as usize;
        let a = EDDY * (dt / steps as f64) / (self.h * self.h);
        for comp in 0..3 {
            let d = match comp {
                0 => self.du,
                1 => self.dv,
                _ => self.dw,
            };
            for _ in 0..steps {
                let mut out = std::mem::take(&mut self.scratch);
                out.resize(d.len().max(out.len()), 0.0);
                {
                    let (f, open) = match comp {
                        0 => (&self.u, &self.ou),
                        1 => (&self.v, &self.ov),
                        _ => (&self.w, &self.ow),
                    };
                    for k in 0..d.c {
                        for j in 0..d.b {
                            for i in 0..d.a {
                                let c = d.at(i, j, k);
                                if !open[c] {
                                    out[c] = 0.0;
                                    continue;
                                }
                                let mut lap = 0.0;
                                for (di, dj, dk) in [(-1i64, 0i64, 0i64), (1, 0, 0), (0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1)] {
                                    let (ii, jj, kk) = (i as i64 + di, j as i64 + dj, k as i64 + dk);
                                    let nb = if ii < 0 || jj < 0 || kk < 0 || ii >= d.a as i64 || jj >= d.b as i64 || kk >= d.c as i64 {
                                        0.0
                                    } else {
                                        let m = d.at(ii as usize, jj as usize, kk as usize);
                                        if open[m] { f[m] } else { 0.0 }
                                    };
                                    lap += nb - f[c];
                                }
                                out[c] = f[c] + a * lap;
                            }
                        }
                    }
                }
                let target = match comp {
                    0 => &mut self.u,
                    1 => &mut self.v,
                    _ => &mut self.w,
                };
                target.copy_from_slice(&out[..d.len()]);
                self.scratch = out;
            }
        }
    }

    /// Make the face velocities satisfy the mass sources: supply cells push air in, exhaust cells take it out.
    fn project(&mut self, supply_rate: f64, exhaust_rate: f64) {
        let h = self.h;
        for k in 0..self.nz {
            for j in 0..self.ny {
                for i in 0..self.nx {
                    let c = self.idx(i, j, k);
                    if !self.fluid(c) {
                        self.div[c] = 0.0;
                        continue;
                    }
                    let d = (self.u[self.du.at(i + 1, j, k)] - self.u[self.du.at(i, j, k)] + self.v[self.dv.at(i, j + 1, k)] - self.v[self.dv.at(i, j, k)] + self.w[self.dw.at(i, j, k + 1)] - self.w[self.dw.at(i, j, k)]) / h;
                    let source = match self.kind[c] {
                        Cell::Supply => supply_rate,
                        Cell::Exhaust => -exhaust_rate,
                        _ => 0.0,
                    };
                    self.div[c] = d - source;
                }
            }
        }
        for _ in 0..PROJECTION_ITERATIONS {
            for k in 0..self.nz {
                for j in 0..self.ny {
                    for i in 0..self.nx {
                        let c = self.idx(i, j, k);
                        let n_open = self.open_faces[c];
                        if n_open == 0 || !self.fluid(c) {
                            continue;
                        }
                        let mut sum = 0.0;
                        if self.ou[self.du.at(i, j, k)] {
                            sum += self.pressure[c - 1];
                        }
                        if self.ou[self.du.at(i + 1, j, k)] {
                            sum += self.pressure[c + 1];
                        }
                        if self.ov[self.dv.at(i, j, k)] {
                            sum += self.pressure[c - self.nx];
                        }
                        if self.ov[self.dv.at(i, j + 1, k)] {
                            sum += self.pressure[c + self.nx];
                        }
                        if self.ow[self.dw.at(i, j, k)] {
                            sum += self.pressure[c - self.nx * self.ny];
                        }
                        if self.ow[self.dw.at(i, j, k + 1)] {
                            sum += self.pressure[c + self.nx * self.ny];
                        }
                        let new = (sum - h * h * self.div[c]) / n_open as f64;
                        self.pressure[c] += SOR * (new - self.pressure[c]);
                    }
                }
            }
        }
        // Pressure is only defined up to a constant; keep it centred so it cannot drift.
        let fluid_n = self.cells().max(1) as f64;
        let mean = self.pressure.iter().zip(&self.kind).filter(|(_, k)| **k != Cell::Solid).map(|(p, _)| *p).sum::<f64>() / fluid_n;
        for (p, k) in self.pressure.iter_mut().zip(&self.kind) {
            *p = if *k != Cell::Solid { *p - mean } else { 0.0 };
        }
        for k in 0..self.nz {
            for j in 0..self.ny {
                for i in 1..self.nx {
                    let f = self.du.at(i, j, k);
                    if self.ou[f] {
                        let c = self.idx(i, j, k);
                        self.u[f] -= (self.pressure[c] - self.pressure[c - 1]) / h;
                    }
                }
            }
        }
        for k in 0..self.nz {
            for j in 1..self.ny {
                for i in 0..self.nx {
                    let f = self.dv.at(i, j, k);
                    if self.ov[f] {
                        let c = self.idx(i, j, k);
                        self.v[f] -= (self.pressure[c] - self.pressure[c - self.nx]) / h;
                    }
                }
            }
        }
        for k in 1..self.nz {
            for j in 0..self.ny {
                for i in 0..self.nx {
                    let f = self.dw.at(i, j, k);
                    if self.ow[f] {
                        let c = self.idx(i, j, k);
                        self.w[f] -= (self.pressure[c] - self.pressure[c - self.nx * self.ny]) / h;
                    }
                }
            }
        }
    }

    /// Advect a cell-centred scalar field and diffuse it between fluid cells.
    fn transport(&mut self, field: usize, dt: f64) {
        let h = self.h;
        let n = self.kind.len();
        let mut out = std::mem::take(&mut self.scratch);
        out.resize(n.max(out.len()), 0.0);
        {
            let src = match field {
                0 => &self.t,
                1 => &self.q,
                2 => &self.co2,
                _ => &self.smoke,
            };
            for k in 0..self.nz {
                for j in 0..self.ny {
                    for i in 0..self.nx {
                        let c = self.idx(i, j, k);
                        if !self.fluid(c) {
                            out[c] = src[c];
                            continue;
                        }
                        let (vu, vv, vw) = self.cell_velocity(i, j, k);
                        let (x, y, z) = ((i as f64 + 0.5) * h - vu * dt, (j as f64 + 0.5) * h - vv * dt, (k as f64 + 0.5) * h - vw * dt);
                        out[c] = self.scalar(src, x, y, z);
                    }
                }
            }
        }
        {
            let target = match field {
                0 => &mut self.t,
                1 => &mut self.q,
                2 => &mut self.co2,
                _ => &mut self.smoke,
            };
            target.copy_from_slice(&out[..n]);
        }
        let limit = h * h / (6.5 * EDDY);
        let steps = (dt / limit).ceil().max(1.0) as usize;
        let a = EDDY * (dt / steps as f64) / (h * h);
        for _ in 0..steps {
            {
                let src = match field {
                    0 => &self.t,
                    1 => &self.q,
                    2 => &self.co2,
                    _ => &self.smoke,
                };
                for k in 0..self.nz {
                    for j in 0..self.ny {
                        for i in 0..self.nx {
                            let c = self.idx(i, j, k);
                            if !self.fluid(c) {
                                out[c] = src[c];
                                continue;
                            }
                            let mut lap = 0.0;
                            if self.ou[self.du.at(i, j, k)] {
                                lap += src[c - 1] - src[c];
                            }
                            if self.ou[self.du.at(i + 1, j, k)] {
                                lap += src[c + 1] - src[c];
                            }
                            if self.ov[self.dv.at(i, j, k)] {
                                lap += src[c - self.nx] - src[c];
                            }
                            if self.ov[self.dv.at(i, j + 1, k)] {
                                lap += src[c + self.nx] - src[c];
                            }
                            if self.ow[self.dw.at(i, j, k)] {
                                lap += src[c - self.nx * self.ny] - src[c];
                            }
                            if self.ow[self.dw.at(i, j, k + 1)] {
                                lap += src[c + self.nx * self.ny] - src[c];
                            }
                            out[c] = src[c] + a * lap;
                        }
                    }
                }
            }
            let target = match field {
                0 => &mut self.t,
                1 => &mut self.q,
                2 => &mut self.co2,
                _ => &mut self.smoke,
            };
            target.copy_from_slice(&out[..n]);
        }
        self.scratch = out;
    }

    /// Advance the cabin air by `dt`.
    pub fn step(&mut self, inp: &Inputs, dt: f64) -> Outputs {
        let h = self.h;
        let cell_vol = h * h * h;
        let rho = inp.density.max(0.1);
        let fluid_cells = self.cells().max(1) as f64;
        let vol_flow = inp.supply_flow.max(0.0) / rho;
        let supply_rate = vol_flow / (self.supply_cells as f64 * cell_vol);
        let exhaust_rate = vol_flow / (self.exhaust_cells as f64 * cell_vol);
        let t_ref = self.t.iter().zip(&self.kind).filter(|(_, k)| **k != Cell::Solid).map(|(t, _)| *t).sum::<f64>() / fluid_cells;

        // Buoyancy on the vertical faces from the cells either side.
        for k in 1..self.nz {
            for j in 0..self.ny {
                for i in 0..self.nx {
                    let f = self.dw.at(i, j, k);
                    if self.ow[f] {
                        let (below, above) = (self.idx(i, j, k - 1), self.idx(i, j, k));
                        let tf = 0.5 * (self.t[below] + self.t[above]);
                        self.w[f] += G * (tf - t_ref) / t_ref * dt;
                    }
                }
            }
        }
        // Diffuser jets: the slots blow inboard and down.
        let jet = (supply_rate * h).min(JET_MAX);
        for k in 0..self.nz {
            for j in 0..self.ny {
                for i in 0..self.nx {
                    let c = self.idx(i, j, k);
                    if self.kind[c] != Cell::Supply {
                        continue;
                    }
                    let inboard = (j as f64 + 0.5) < self.ny as f64 / 2.0;
                    let fv = if inboard { self.dv.at(i, j + 1, k) } else { self.dv.at(i, j, k) };
                    if self.ov[fv] {
                        self.v[fv] = if inboard { 0.7 * jet } else { -0.7 * jet };
                    }
                    let fw = self.dw.at(i, j, k);
                    if self.ow[fw] {
                        self.w[fw] = -0.7 * jet;
                    }
                }
            }
        }
        self.advect_velocity(dt);
        self.diffuse_velocity(dt);
        self.project(supply_rate, exhaust_rate);
        let smoky = inp.smoke_rate > 0.0 || self.smoke.iter().any(|s| *s > 1e-6);
        for field in 0..if smoky { 4 } else { 3 } {
            self.transport(field, dt);
        }

        // Sources and sinks in each cell.
        let cell_mass = rho * cell_vol;
        let cell_c = cell_mass * CP;
        let seat_heat = (inp.occupants * PAX_SENSIBLE + inp.heat) / self.seat_cells as f64;
        let seat_water = inp.occupants * PAX_LATENT / LATENT / self.seat_cells as f64;
        let seat_co2 = inp.occupants * PAX_CO2 / self.seat_cells as f64;
        let faces: f64 = self.wall_faces.iter().map(|f| *f as f64).sum::<f64>().max(1.0);
        let skin_per_face = inp.skin_ua / faces;
        let furn_per_cell = inp.furnishing_ua / fluid_cells;
        let supply_mass = inp.supply_flow.max(0.0) / self.supply_cells as f64;
        let (mut wall_loss, mut furn_gain) = (0.0, 0.0);
        let (mut ex_t, mut ex_q, mut ex_c) = (0.0, 0.0, 0.0);
        for c in 0..self.kind.len() {
            if !self.fluid(c) {
                continue;
            }
            let mut heat = furn_per_cell * (inp.furnishing_t - self.t[c]);
            furn_gain += heat;
            if self.seat[c] {
                heat += seat_heat;
                self.q[c] += seat_water * dt / cell_mass;
                self.co2[c] += seat_co2 * dt / cell_mass * 1e6 / 1.52;
            }
            // Wall loss implicit so a cold skin cannot drive a cell past the skin temperature.
            let wall_g = skin_per_face * self.wall_faces[c] as f64;
            let t_new = (cell_c * self.t[c] + dt * (heat + wall_g * inp.skin_t)) / (cell_c + dt * wall_g);
            wall_loss += wall_g * (t_new - inp.skin_t);
            self.t[c] = t_new;
            match self.kind[c] {
                Cell::Supply => {
                    let mix = (supply_mass * dt / cell_mass).min(1.0);
                    self.t[c] += (inp.supply_t - self.t[c]) * mix;
                    self.q[c] += (inp.supply_q - self.q[c]) * mix;
                    self.co2[c] += (inp.supply_co2 - self.co2[c]) * mix;
                }
                Cell::Exhaust => {
                    ex_t += self.t[c];
                    ex_q += self.q[c];
                    ex_c += self.co2[c];
                    // Grilles and extract fans take smoke with the air they draw.
                    let drawn = (supply_mass + inp.extract / self.exhaust_cells as f64) * dt / cell_mass;
                    self.smoke[c] *= (1.0 - drawn).clamp(0.0, 1.0);
                }
                _ => {}
            }
        }
        if inp.smoke_rate > 0.0 {
            let (sx, sy, sz) = inp.smoke_at;
            let at = |f: f64, n: usize| ((f.clamp(0.0, 1.0) * n as f64) as usize).min(n - 1);
            let (i, j) = (at(sx, self.nx), at(sy, self.ny));
            let k = (at(sz, self.nz)..self.nz).chain((0..at(sz, self.nz)).rev()).find(|&k| self.fluid(self.idx(i, j, k)));
            if let Some(k) = k {
                let c = self.idx(i, j, k);
                self.smoke[c] += inp.smoke_rate * 1e6 * dt / cell_vol;
            }
        }
        // Smoke settles out on surfaces slowly.
        let settle = (1.0 - dt / 1800.0).max(0.0);
        self.smoke.iter_mut().for_each(|s| *s *= settle);
        self.bound();
        let ex = self.exhaust_cells as f64;
        self.report(ex_t / ex, ex_q / ex, ex_c / ex, wall_loss, furn_gain)
    }

    fn report(&self, exhaust_t: f64, exhaust_q: f64, exhaust_co2: f64, wall_loss: f64, furnishing_gain: f64) -> Outputs {
        let h = self.h;
        let layer = |z: f64| ((z / h - 0.5).round().max(0.0) as usize).min(self.nz - 1);
        let (kh, ka) = (layer(HEAD), layer(ANKLE));
        let avg_layer = |k: usize| {
            let (mut s, mut n) = (0.0, 0.0);
            for j in 0..self.ny {
                for i in 0..self.nx {
                    let c = self.idx(i, j, k);
                    if self.fluid(c) {
                        s += self.t[c];
                        n += 1.0;
                    }
                }
            }
            if n > 0.0 { s / n } else { f64::NAN }
        };
        let top = (0..self.nz).rev().find(|&k| (0..self.ny).any(|j| self.fluid(self.idx(0, j, k)))).unwrap_or(self.nz - 1);
        let mut o = Outputs { min_t: f64::INFINITY, max_t: f64::NEG_INFINITY, ..Default::default() };
        let mut n = 0.0f64;
        for k in 0..self.nz {
            for j in 0..self.ny {
                for i in 0..self.nx {
                    let c = self.idx(i, j, k);
                    if !self.fluid(c) {
                        continue;
                    }
                    n += 1.0;
                    o.mean_t += self.t[c];
                    o.mean_q += self.q[c];
                    o.mean_co2 += self.co2[c];
                    o.min_t = o.min_t.min(self.t[c]);
                    o.max_t = o.max_t.max(self.t[c]);
                    let (vu, vv, vw) = self.cell_velocity(i, j, k);
                    let speed = (vu * vu + vv * vv + vw * vw).sqrt();
                    o.mean_speed += speed;
                    if self.seat[c] {
                        o.draught = o.draught.max(speed);
                        o.seat_co2_max = o.seat_co2_max.max(self.co2[c]);
                    }
                }
            }
        }
        let n = n.max(1.0);
        o.mean_t /= n;
        o.mean_q /= n;
        o.mean_co2 /= n;
        o.mean_speed /= n;
        o.head_t = avg_layer(kh);
        o.ankle_t = avg_layer(ka);
        o.ceiling_t = avg_layer(top);
        if o.head_t.is_nan() {
            o.head_t = o.mean_t;
        }
        if o.ankle_t.is_nan() {
            o.ankle_t = o.mean_t;
        }
        if o.ceiling_t.is_nan() {
            o.ceiling_t = o.mean_t;
        }
        o.sensor_t = self.t[self.sensor];
        o.smoke_detector = self.smoke[self.detector];
        let fluid: Vec<f64> = self.smoke.iter().zip(&self.kind).filter(|(_, k)| **k != Cell::Solid).map(|(s, _)| *s).collect();
        o.smoke_max = fluid.iter().cloned().fold(0.0, f64::max);
        o.smoke_mean = fluid.iter().sum::<f64>() / fluid.len().max(1) as f64;
        o.exhaust_t = exhaust_t;
        o.exhaust_q = exhaust_q;
        o.exhaust_co2 = exhaust_co2;
        o.wall_loss = wall_loss;
        o.furnishing_gain = furnishing_gain;
        o
    }

    /// Temperatures across the section at the middle of the zone, for diagrams: rows from floor up.
    pub fn section(&self) -> Vec<Vec<Option<f64>>> {
        let i = self.nx / 2;
        (0..self.nz).map(|k| (0..self.ny).map(|j| {
            let c = self.idx(i, j, k);
            self.fluid(c).then(|| self.t[c])
        }).collect()).collect()
    }

    /// Still air at one temperature, humidity and CO2.
    pub fn settle(&mut self, t: f64, q: f64, co2: f64) {
        let ok = |x: f64, or: f64| if x.is_finite() { x } else { or };
        self.u.iter_mut().for_each(|x| *x = 0.0);
        self.v.iter_mut().for_each(|x| *x = 0.0);
        self.w.iter_mut().for_each(|x| *x = 0.0);
        self.pressure.iter_mut().for_each(|x| *x = 0.0);
        self.t.iter_mut().for_each(|x| *x = ok(t, 295.0));
        self.q.iter_mut().for_each(|x| *x = ok(q, 0.006));
        self.co2.iter_mut().for_each(|x| *x = ok(co2, 420.0));
        self.smoke.iter_mut().for_each(|x| *x = 0.0);
    }

    /// Keep every field inside what cabin air can physically do.
    fn bound(&mut self) {
        for f in [&mut self.u, &mut self.v, &mut self.w] {
            for x in f.iter_mut() {
                *x = if x.is_finite() { x.clamp(-15.0, 15.0) } else { 0.0 };
            }
        }
        for x in self.t.iter_mut() {
            *x = if x.is_finite() { x.clamp(180.0, 420.0) } else { 295.0 };
        }
        for x in self.q.iter_mut() {
            *x = if x.is_finite() { x.clamp(0.0, 0.05) } else { 0.006 };
        }
        for x in self.smoke.iter_mut() {
            *x = if x.is_finite() { x.clamp(0.0, 1e5) } else { 0.0 };
        }
        for x in self.co2.iter_mut() {
            *x = if x.is_finite() { x.clamp(0.0, 1e5) } else { 420.0 };
        }
        for x in self.pressure.iter_mut() {
            if !x.is_finite() {
                *x = 0.0;
            }
        }
    }

    pub fn state(&self, v: &mut Vec<f64>) {
        for f in [&self.u, &self.v, &self.w, &self.t, &self.q, &self.co2, &self.smoke, &self.pressure] {
            v.extend_from_slice(f);
        }
    }

    pub fn restore(&mut self, it: &mut impl Iterator<Item = f64>) {
        for f in [&mut self.u, &mut self.v, &mut self.w, &mut self.t, &mut self.q, &mut self.co2, &mut self.smoke, &mut self.pressure] {
            for x in f.iter_mut() {
                *x = it.next().unwrap_or(*x);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape() -> Shape {
        Shape { section: Section::Circle { radius: 3.0 }, length: 4.0, floor: -1.2, ceiling: 1.4, cell: 0.4, seats_abreast: 6.0, bins: true }
    }

    fn still(occupants: f64, supply: f64) -> Inputs {
        Inputs { supply_flow: supply, supply_t: 288.0, supply_q: 0.004, supply_co2: 420.0, smoke_rate: 0.0, smoke_at: (0.5, 0.5, 0.5), extract: 0.0, occupants, heat: 0.0, skin_t: 295.0, skin_ua: 0.0, density: 1.0, furnishing_t: 295.0, furnishing_ua: 0.0 }
    }

    #[test]
    fn the_grid_follows_the_section_with_supply_and_exhaust() {
        let g = Grid::new(shape(), 295.0, 0.006, 600.0);
        assert!(g.supply_cells > 0 && g.exhaust_cells > 0 && g.seat_cells > 0);
        assert!(g.cells() < g.nx * g.ny * g.nz, "the section is not a box");
    }

    #[test]
    fn every_section_shape_builds_a_grid() {
        for section in [Section::Ellipse { width: 7.0, height: 8.4 }, Section::Rectangle { width: 2.4, height: 1.9 }, Section::parse("polygon", 0.0, 0.0, 0.0, "-1.2,-0.9 1.2,-0.9 1.3,0.6 0,1.0 -1.3,0.6")] {
            let floor = section.extent().2 + 0.1;
            let g = Grid::new(Shape { section, length: 3.0, floor, ceiling: 5.0, cell: 0.25, seats_abreast: 2.0, bins: false }, 295.0, 0.006, 600.0);
            assert!(g.supply_cells > 0 && g.exhaust_cells > 0 && g.cells() > 10);
        }
    }

    #[test]
    fn passengers_raise_plumes_and_warm_air_collects_under_the_ceiling() {
        let mut g = Grid::new(shape(), 295.0, 0.006, 600.0);
        let mut o = Outputs::default();
        for _ in 0..240 {
            o = g.step(&still(20.0, 0.0), 0.5);
        }
        assert!(o.mean_t > 295.5, "{:.2}", o.mean_t);
        assert!(o.ceiling_t > o.ankle_t + 0.2, "ceiling {:.2} ankle {:.2}", o.ceiling_t, o.ankle_t);
        assert!(o.mean_speed > 0.01, "convection moves the air: {:.3} m/s", o.mean_speed);
        assert!(o.mean_co2 > 600.0);
    }

    #[test]
    fn smoke_drifts_to_the_ceiling_detector() {
        let mut g = Grid::new(shape(), 295.0, 0.006, 600.0);
        let mut inp = still(10.0, 0.4);
        inp.smoke_rate = 2e-5;
        inp.smoke_at = (0.2, 0.3, 0.1);
        let mut first = None;
        for s in 0..240 {
            let o = g.step(&inp, 0.5);
            if first.is_none() && o.smoke_detector > 4.0 {
                first = Some(s);
            }
        }
        assert!(first.is_some(), "the detector sees it");
        assert!(first.unwrap() > 2, "but not at once");
    }

    #[test]
    fn supply_and_exhaust_balance_and_the_cabin_takes_the_supply_temperature() {
        let mut g = Grid::new(shape(), 300.0, 0.006, 600.0);
        let mut o = Outputs::default();
        for _ in 0..600 {
            o = g.step(&still(0.0, 0.5), 0.5);
        }
        assert!((o.mean_t - 288.0).abs() < 1.0, "{:.2}", o.mean_t);
        assert!(o.draught > 0.02 && o.draught < 3.0, "jets reach the seats: {:.2} m/s", o.draught);
    }
}
