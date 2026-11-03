//! EasyAcft aircraft files: loader, schemas, expressions, diagnostics.

pub mod diag;
pub mod expr;
pub mod model;
pub mod schema;

pub use diag::{Diagnostic, Severity, Sources};
pub use expr::{Program, Type};
pub use model::{load, load_str, Model};
pub use schema::{check, opt, req, Catalog, Checked, CheckedPart, Kind, PartSpec, Value};
