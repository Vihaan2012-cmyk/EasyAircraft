//! Whole-state snapshots as tagged little-endian `f64` sections, for save/load, replay and test
//! fixtures.

const MAGIC: &[u8; 8] = b"EACFSNP1";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    sections: Vec<(String, Vec<f64>)>,
}

impl Snapshot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, name: &str, values: Vec<f64>) {
        self.sections.retain(|(n, _)| n != name);
        self.sections.push((name.to_owned(), values));
    }

    pub fn get(&self, name: &str) -> Option<&[f64]> {
        self.sections.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_slice())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend((self.sections.len() as u32).to_le_bytes());
        for (name, values) in &self.sections {
            out.extend((name.len() as u32).to_le_bytes());
            out.extend(name.as_bytes());
            out.extend((values.len() as u32).to_le_bytes());
            for v in values {
                out.extend(v.to_le_bytes());
            }
        }
        out
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let mut at = 0;
        let mut take = |n: usize| -> Option<&[u8]> {
            let s = bytes.get(at..at + n)?;
            at += n;
            Some(s)
        };
        if take(8)? != MAGIC {
            return None;
        }
        let u32_of = |s: &[u8]| u32::from_le_bytes(s.try_into().unwrap()) as usize;
        let count = u32_of(take(4)?);
        let mut sections = Vec::with_capacity(count.min(1 << 16));
        for _ in 0..count {
            let len = u32_of(take(4)?);
            let name = String::from_utf8(take(len)?.to_vec()).ok()?;
            let n = u32_of(take(4)?);
            let mut values = Vec::with_capacity(n.min(1 << 24));
            for _ in 0..n {
                values.push(f64::from_le_bytes(take(8)?.try_into().unwrap()));
            }
            sections.push((name, values));
        }
        Some(Self { sections })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_damage() {
        let mut s = Snapshot::new();
        s.put("electrical", vec![28.0, -1.5, f64::MAX]);
        s.put("fuel", vec![]);
        s.put("electrical", vec![1.0]);
        let bytes = s.to_bytes();
        let back = Snapshot::from_bytes(&bytes).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.get("electrical"), Some(&[1.0][..]));
        assert!(Snapshot::from_bytes(&bytes[..bytes.len() - 3]).is_none());
        assert!(Snapshot::from_bytes(b"not a snapshot").is_none());
    }
}
