//! IDE queries answered against an [`Analysis`].
//!
//! All three queries work off the resolved occurrence list in the symbol table:
//! go to definition follows an occurrence to its binding, find references
//! gathers every occurrence of that binding, and hover reads the binding's
//! metadata.

use crate::incremental::Analysis;
use crate::resolver::{BindingKind, Occurrence};
use crate::span::Span;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Definition {
    pub span: Span,
    pub name: String,
    pub kind: BindingKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub span: Span,
    pub is_decl: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hover {
    pub name: String,
    pub kind: BindingKind,
    pub decl_span: Span,
    pub arity: Option<usize>,
}

/// Find the identifier occurrence at `pos`, preferring one that strictly
/// contains the position over one it merely touches at an edge.
fn occurrence_at(analysis: &Analysis, pos: u32) -> Option<&Occurrence> {
    let occ = &analysis.symbols().occurrences;
    occ.iter()
        .find(|o| o.span.contains(pos))
        .or_else(|| occ.iter().find(|o| o.span.touches(pos)))
}

impl Analysis {
    /// Resolve the definition of the symbol at `pos`.
    pub fn go_to_definition(&self, pos: u32) -> Option<Definition> {
        let occ = occurrence_at(self, pos)?;
        let id = occ.binding?;
        let b = &self.symbols().bindings[id];
        Some(Definition {
            span: b.decl_span,
            name: b.name.clone(),
            kind: b.kind,
        })
    }

    /// Every reference to the symbol at `pos`, including its declaration, sorted
    /// by position.
    pub fn find_references(&self, pos: u32) -> Vec<Reference> {
        let Some(occ) = occurrence_at(self, pos) else {
            return Vec::new();
        };
        let Some(id) = occ.binding else {
            return Vec::new();
        };
        let mut refs: Vec<Reference> = self
            .symbols()
            .occurrences
            .iter()
            .filter(|o| o.binding == Some(id))
            .map(|o| Reference {
                span: o.span,
                is_decl: o.is_decl,
            })
            .collect();
        refs.sort_by_key(|r| (r.span.start, r.span.end));
        refs
    }

    /// Hover information for the symbol at `pos`.
    pub fn hover(&self, pos: u32) -> Option<Hover> {
        let occ = occurrence_at(self, pos)?;
        let id = occ.binding?;
        let b = &self.symbols().bindings[id];
        Some(Hover {
            name: b.name.clone(),
            kind: b.kind,
            decl_span: b.decl_span,
            arity: b.arity,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_to_definition_and_references() {
        let src = "let x = 1;\nlet y = x + x;\n";
        let a = Analysis::new(src);
        let first_use = src.find("x + x").unwrap() as u32;
        let def = a.go_to_definition(first_use).unwrap();
        assert_eq!(def.span, Span::new(4, 5));
        assert_eq!(def.name, "x");
        let refs = a.find_references(first_use);
        // one decl plus two uses
        assert_eq!(refs.len(), 3);
        assert_eq!(refs.iter().filter(|r| r.is_decl).count(), 1);
    }

    #[test]
    fn hover_reports_kind_and_arity() {
        let a = Analysis::new("fn add(a, b) { a + b }");
        let pos = a.text().find("add").unwrap() as u32;
        let h = a.hover(pos).unwrap();
        assert_eq!(h.kind, BindingKind::Fn);
        assert_eq!(h.arity, Some(2));
    }
}
