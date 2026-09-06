//! Diagnostics produced by parsing and name resolution.

use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warning,
}

/// The class of a diagnostic. Kept separate from the human message so tools can
/// group and filter without string matching.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DiagKind {
    ParseError,
    UnexpectedEof,
    UnresolvedName,
    Redefinition,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub severity: Severity,
    pub kind: DiagKind,
    pub message: String,
}

impl Diagnostic {
    pub fn error(span: Span, kind: DiagKind, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            span,
            severity: Severity::Error,
            kind,
            message: message.into(),
        }
    }

    fn sort_key(&self) -> (u32, u32, DiagKind, &str) {
        (
            self.span.start,
            self.span.end,
            self.kind,
            self.message.as_str(),
        )
    }
}

impl PartialOrd for Diagnostic {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Diagnostic {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.sort_key().cmp(&other.sort_key())
    }
}

/// Sort diagnostics into a stable, deterministic order. Every producer routes
/// through here so the same program always yields the same diagnostic sequence.
pub fn sorted(mut diags: Vec<Diagnostic>) -> Vec<Diagnostic> {
    diags.sort();
    diags
}
