//! The plugin's exported catalogue of failures, components, breakers, ECAM alerts and MEL entries.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(BTreeMap<String, Json>),
}

impl Json {
    pub fn get(&self, k: &str) -> &Json {
        static NULL: Json = Json::Null;
        match self {
            Json::Obj(m) => m.get(k).unwrap_or(&NULL),
            _ => &NULL,
        }
    }
    pub fn str(&self) -> &str {
        if let Json::Str(s) = self {
            s
        } else {
            ""
        }
    }
    pub fn num(&self) -> f64 {
        if let Json::Num(n) = self {
            *n
        } else {
            0.0
        }
    }
    pub fn arr(&self) -> &[Json] {
        if let Json::Arr(a) = self {
            a
        } else {
            &[]
        }
    }
}

struct P<'a> {
    s: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.s.get(self.i) {
            Some(b'{') => {
                self.i += 1;
                let mut m = BTreeMap::new();
                loop {
                    self.ws();
                    if self.s.get(self.i) == Some(&b'}') {
                        self.i += 1;
                        return Ok(Json::Obj(m));
                    }
                    let Json::Str(k) = self.value()? else { return Err(format!("key expected at {}", self.i)) };
                    self.ws();
                    if self.s.get(self.i) != Some(&b':') {
                        return Err(format!("`:` expected at {}", self.i));
                    }
                    self.i += 1;
                    let v = self.value()?;
                    m.insert(k, v);
                    self.ws();
                    if self.s.get(self.i) == Some(&b',') {
                        self.i += 1;
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut a = Vec::new();
                loop {
                    self.ws();
                    if self.s.get(self.i) == Some(&b']') {
                        self.i += 1;
                        return Ok(Json::Arr(a));
                    }
                    a.push(self.value()?);
                    self.ws();
                    if self.s.get(self.i) == Some(&b',') {
                        self.i += 1;
                    }
                }
            }
            Some(b'"') => {
                self.i += 1;
                let mut out = String::new();
                while let Some(&c) = self.s.get(self.i) {
                    self.i += 1;
                    match c {
                        b'"' => return Ok(Json::Str(out)),
                        b'\\' => {
                            let e = self.s.get(self.i).copied().unwrap_or(b'"');
                            self.i += 1;
                            match e {
                                b'n' => out.push('\n'),
                                b't' => out.push('\t'),
                                b'u' => {
                                    let hex = std::str::from_utf8(&self.s[self.i..(self.i + 4).min(self.s.len())]).unwrap_or("0");
                                    self.i += 4;
                                    out.push(char::from_u32(u32::from_str_radix(hex, 16).unwrap_or(63)).unwrap_or('?'));
                                }
                                other => out.push(other as char),
                            }
                        }
                        _ => {
                            let start = self.i - 1;
                            let len = if c < 0x80 { 1 } else if c >= 0xF0 { 4 } else if c >= 0xE0 { 3 } else { 2 };
                            let end = (start + len).min(self.s.len());
                            out.push_str(std::str::from_utf8(&self.s[start..end]).unwrap_or("?"));
                            self.i = end;
                        }
                    }
                }
                Err("unterminated string".into())
            }
            Some(b't') => {
                self.i += 4;
                Ok(Json::Bool(true))
            }
            Some(b'f') => {
                self.i += 5;
                Ok(Json::Bool(false))
            }
            Some(b'n') => {
                self.i += 4;
                Ok(Json::Null)
            }
            Some(_) => {
                let start = self.i;
                while self.i < self.s.len() && matches!(self.s[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i]).ok().and_then(|t| t.parse().ok()).map(Json::Num).ok_or_else(|| format!("bad value at {start}"))
            }
            None => Err("unexpected end".into()),
        }
    }
}

pub fn parse(text: &str) -> Result<Json, String> {
    P { s: text.as_bytes(), i: 0 }.value()
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Breaker {
    pub id: String,
    pub name: String,
    pub bus: String,
    pub rating: f64,
    pub sspc: bool,
    pub consumer: String,
    pub ata: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comp {
    pub id: String,
    pub name: String,
    pub ata: u32,
    pub failures: Vec<u64>,
    pub params: Vec<(String, f64, String)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalogue {
    pub failures: Vec<(u64, String, u32, String)>,
    pub breakers: Vec<Breaker>,
    pub alerts: Vec<(String, String, String, Vec<u64>)>,
    pub components: Vec<Comp>,
    pub mel: Vec<(String, Vec<u64>)>,
    pub chapters: Vec<(u32, String)>,
}

/// `DC_ESS` to `DcEss`, the plugin's bus naming.
pub fn bus_name(s: &str) -> String {
    s.split('_').map(|p| p[..1].to_uppercase() + &p[1..].to_ascii_lowercase()).collect()
}

pub fn read(text: &str) -> Result<Catalogue, String> {
    let j = parse(text)?;
    let mut c = Catalogue::default();
    for f in j.get("failures").arr() {
        c.failures.push((f.get("id").num() as u64, f.get("name").str().to_owned(), f.get("ataChapterNumber").num() as u32, f.get("component").str().to_owned()));
    }
    for b in j.get("breakers").arr() {
        c.breakers.push(Breaker {
            id: b.get("id").str().to_owned(),
            name: b.get("name").str().to_owned(),
            bus: bus_name(b.get("bus").str()),
            rating: b.get("ratingAmperes").num(),
            sspc: b.get("kind").str() == "solidStatePowerController",
            consumer: b.get("consumer").str().to_owned(),
            ata: b.get("ataChapterNumber").num() as u32,
        });
    }
    for a in j.get("alerts").arr() {
        c.alerts.push((a.get("key").str().to_owned(), a.get("title").str().to_owned(), a.get("level").str().to_owned(), a.get("failures").arr().iter().map(|x| x.num() as u64).collect()));
    }
    for x in j.get("components").arr() {
        let params = x.get("parameters").arr().iter().map(|p| (p.get("name").str().to_owned(), p.get("healthyValue").num(), p.get("meaning").str().to_owned())).collect();
        c.components.push(Comp { id: x.get("id").str().to_owned(), name: x.get("name").str().to_owned(), ata: x.get("ataChapterNumber").num() as u32, failures: x.get("failures").arr().iter().map(|f| f.num() as u64).collect(), params });
    }
    for m in j.get("melEntries").arr() {
        c.mel.push((m.get("melReference").str().to_owned(), m.get("failures").arr().iter().map(|f| f.num() as u64).collect()));
    }
    for ch in j.get("chapters").arr() {
        c.chapters.push((ch.get("number").num() as u32, ch.get("name").str().to_owned()));
    }
    if c.failures.is_empty() && c.breakers.is_empty() {
        return Err("no failures or breakers in that catalogue".into());
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_small_catalogue() {
        let t = r#"{"failures":[{"id":21000,"name":"Rapid Decompression","ataChapterNumber":21,"component":"Cabin"}],"breakers":[{"id":"acars-mu","name":"ACARS MU","bus":"DC_ESS","ratingAmperes":3.0,"kind":"solidStatePowerController","consumer":"ACARS","ataChapterNumber":23}],"alerts":[{"key":"APU_FIRE","title":"APU FIRE","level":"warning","failures":[21000]}],"components":[{"id":"24_elec.bus.DC_ESS","name":"DC ESS bus","failures":[21000],"parameters":[{"name":"short_to_ground","healthyValue":0.0,"meaning":"m"}]}],"melEntries":[],"chapters":[{"number":21,"name":"21 Air é"}]}"#;
        let c = read(t).unwrap();
        assert_eq!((c.failures.len(), c.breakers[0].bus.as_str(), c.alerts[0].3[0], c.components[0].params.len()), (1, "DcEss", 21000, 1));
        assert_eq!(c.chapters[0].1, "21 Air é");
    }
}
