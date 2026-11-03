//! rustc-style diagnostics.

use std::fmt::Write;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub file: usize,
    pub span: Range<usize>,
    pub label: String,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>, file: usize, span: Range<usize>) -> Self {
        Self { severity: Severity::Error, code, message: message.into(), file, span, label: String::new(), help: None }
    }

    pub fn warning(code: &'static str, message: impl Into<String>, file: usize, span: Range<usize>) -> Self {
        Self { severity: Severity::Warning, ..Self::error(code, message, file, span) }
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = label.into();
        self
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}

/// The files a set of diagnostics points into.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    pub files: Vec<(String, String)>,
}

impl Sources {
    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> usize {
        self.files.push((name.into(), text.into()));
        self.files.len() - 1
    }

    fn line_col(text: &str, at: usize) -> (usize, usize, Range<usize>) {
        let at = at.min(text.len());
        let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
        let end = text[at..].find('\n').map_or(text.len(), |i| at + i);
        let line = text[..start].matches('\n').count() + 1;
        (line, text[start..at].chars().count() + 1, start..end)
    }

    pub fn render(&self, d: &Diagnostic) -> String {
        let mut out = String::new();
        let kind = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let _ = writeln!(out, "{kind}[{}]: {}", d.code, d.message);
        if let Some((name, text)) = self.files.get(d.file) {
            let (line, col, range) = Self::line_col(text, d.span.start);
            let src = text[range.clone()].trim_end_matches('\r');
            let gutter = line.to_string().len();
            let pad = " ".repeat(gutter);
            let width = text[d.span.start.min(text.len())..d.span.end.min(range.end).max(d.span.start.min(text.len()))].chars().count().max(1);
            let _ = writeln!(out, "{pad}--> {name}:{line}:{col}");
            let _ = writeln!(out, "{pad} |");
            let _ = writeln!(out, "{line} | {src}");
            let label = if d.label.is_empty() { String::new() } else { format!(" {}", d.label) };
            let _ = writeln!(out, "{pad} | {}{}{label}", " ".repeat(col - 1), "^".repeat(width));
        }
        if let Some(h) = &d.help {
            let _ = writeln!(out, "  = help: {h}");
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_like_rustc() {
        let mut s = Sources::default();
        let f = s.add("systems/elec.toml", "[[electrical.bus]]\nid = \"dc1\"\nnominal = \"28 psi\"\n");
        let at = s.files[f].1.find("\"28 psi\"").unwrap();
        let d = Diagnostic::error("E0103", "`nominal` must be a voltage", f, at..at + 8).label("this is a pressure").help("write it in V, for example \"28 V\"");
        let text = s.render(&d);
        assert_eq!(
            text,
            "error[E0103]: `nominal` must be a voltage\n --> systems/elec.toml:3:11\n  |\n3 | nominal = \"28 psi\"\n  |           ^^^^^^^^ this is a pressure\n  = help: write it in V, for example \"28 V\"\n"
        );
    }
}
