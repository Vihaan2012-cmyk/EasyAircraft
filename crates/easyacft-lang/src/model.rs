//! Aircraft files loaded into part declarations with source spans.

use crate::diag::Sources;
use std::ops::Range;

#[derive(Clone, Debug, PartialEq)]
pub enum Field {
    Str(String),
    Num(f64),
    Bool(bool),
    List(Vec<String>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct FieldDecl {
    pub key: String,
    pub value: Field,
    pub file: usize,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PartDecl {
    pub domain: String,
    pub kind: String,
    pub id: String,
    pub file: usize,
    pub span: Range<usize>,
    pub fields: Vec<FieldDecl>,
}

impl PartDecl {
    pub fn get(&self, key: &str) -> Option<&FieldDecl> {
        self.fields.iter().find(|f| f.key == key)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub dataref: String,
    pub path: String,
    pub write: bool,
    /// The dataref's unit: SI = raw * factor + offset.
    pub factor: f64,
    pub offset: f64,
    pub file: usize,
    pub span: Range<usize>,
}

/// A setting or value of a developer's own part kind, as written: `"L/min"`, `"expr bool"`, `"part electrical.bus"`.
#[derive(Clone, Debug, PartialEq)]
pub struct KindField {
    pub key: String,
    pub ty: String,
    pub default: Option<String>,
    pub help: String,
    pub span: Range<usize>,
}

/// `[kinds.<name>]`: a part kind the developer declares and steps with their own code.
#[derive(Clone, Debug, PartialEq)]
pub struct KindDecl {
    pub name: String,
    pub help: String,
    pub settings: Vec<KindField>,
    pub values: Vec<KindField>,
    /// `ports.<name> = "electrical"`: a connection to one bus, circuit, duct or line.
    pub ports: Vec<KindField>,
    /// `links.<name> = "hydraulic"`: an element between two of them.
    pub links: Vec<KindField>,
    pub file: usize,
    pub span: Range<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct Model {
    pub name: String,
    pub version: String,
    pub parts: Vec<PartDecl>,
    pub bindings: Vec<Binding>,
    pub rates: Vec<(String, f64)>,
    pub kinds: Vec<KindDecl>,
    pub sources: Sources,
}

impl Model {
    pub fn part(&self, id: &str) -> Option<&PartDecl> {
        self.parts.iter().find(|p| p.id == id)
    }
}
