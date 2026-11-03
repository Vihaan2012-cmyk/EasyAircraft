//! Shared helpers for domains.

use crate::catalog::out;
use easyacft_lang::CheckedPart;
use std::collections::HashMap;

/// Output slots of one part by name.
pub struct Slots {
    pub base: HashMap<&'static str, usize>,
}

impl Slots {
    pub fn of(p: &CheckedPart, names: &[&'static str]) -> Self {
        Self { base: names.iter().map(|n| (*n, out(p, n))).collect() }
    }
    pub fn get(&self, s: &[f64], n: &str) -> f64 {
        s[self.base[n]]
    }
    pub fn on(&self, s: &[f64], n: &str) -> bool {
        self.get(s, n) != 0.0
    }
    pub fn put(&self, s: &mut [f64], n: &str, v: f64) {
        s[self.base[n]] = v;
    }
    pub fn flag(&self, s: &mut [f64], n: &str, v: bool) {
        s[self.base[n]] = v as u8 as f64;
    }
}

/// Reads and writes a domain's state as one flat list.
#[derive(Default)]
pub struct Packer(pub Vec<f64>, pub usize);

impl Packer {
    pub fn put(&mut self, v: f64) {
        self.0.push(v);
    }
    pub fn take(&mut self) -> f64 {
        let v = self.0.get(self.1).copied().unwrap_or(0.0);
        self.1 += 1;
        v
    }
}
