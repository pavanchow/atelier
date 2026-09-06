//! Rename refactoring.
//!
//! Renaming a symbol rewrites its declaration and every reference to a new name.
//! The operation is *safe by construction*: after building the candidate text the
//! renamer re-analyses it and refuses the edit unless the binding structure is
//! byte for byte the same as before, so a rename can never silently change which
//! declaration a name resolves to (capture) or leave a name dangling. The gate in
//! `tests/gates.rs` independently confirms this against the reference resolver.

use crate::incremental::Analysis;
use crate::lexer::{lex, TokenKind};
use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameError {
    /// There is no renameable binding at the requested position (nothing there,
    /// or the name does not resolve).
    NotRenameable,
    /// The requested new name is not a single valid identifier.
    InvalidName,
    /// Applying the rename would change name resolution somewhere (capture,
    /// shadowing, or a collision), so it was refused.
    Conflict,
}

/// A single text replacement that makes up a rename.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameEdit {
    pub span: Span,
}

/// The result of a rename: the edits to apply and the resulting text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rename {
    pub edits: Vec<RenameEdit>,
    pub new_text: String,
}

/// True when `name` lexes to exactly one identifier token covering the whole
/// string. Rejects empty names, keywords, numbers, and anything with extra
/// characters or whitespace.
pub fn is_valid_identifier(name: &str) -> bool {
    let toks = lex(name);
    toks.len() == 1
        && toks[0].kind == TokenKind::Ident
        && toks[0].span == Span::new(0, name.len() as u32)
}

/// For each identifier occurrence (sorted by position) the rank of the
/// declaration occurrence it resolves to, or `None` when unresolved. This is a
/// span independent fingerprint of name resolution: two programs with the same
/// shape resolve every occurrence to the correspondingly ranked declaration.
fn resolution_shape(a: &Analysis) -> Vec<Option<usize>> {
    let occ = &a.symbols().occurrences;
    let mut decl_rank = std::collections::HashMap::new();
    for (i, o) in occ.iter().enumerate() {
        if o.is_decl {
            decl_rank.insert(o.span, i);
        }
    }
    occ.iter()
        .map(|o| {
            let decl_span = o.binding.map(|id| a.symbols().bindings[id].decl_span)?;
            decl_rank.get(&decl_span).copied()
        })
        .collect()
}

impl Analysis {
    /// Compute a safe rename of the symbol at `pos` to `new_name`, or an error.
    /// The analysis is not modified; apply [`Rename::new_text`] to commit.
    pub fn rename(&self, pos: u32, new_name: &str) -> Result<Rename, RenameError> {
        if !is_valid_identifier(new_name) {
            return Err(RenameError::InvalidName);
        }

        // Every occurrence of the binding under the cursor, decl and uses.
        let refs = self.find_references(pos);
        if refs.is_empty() {
            return Err(RenameError::NotRenameable);
        }

        // Build the candidate text by replacing each occurrence, right to left so
        // earlier spans keep their offsets while later ones are rewritten.
        let mut spans: Vec<Span> = refs.iter().map(|r| r.span).collect();
        spans.sort_by_key(|s| s.start);
        // A zero width placeholder span (from a missing name during recovery) is
        // not something a user can meaningfully rename.
        if spans.iter().any(super::span::Span::is_empty) {
            return Err(RenameError::NotRenameable);
        }
        let mut new_text = self.text().to_string();
        for span in spans.iter().rev() {
            new_text.replace_range(span.start as usize..span.end as usize, new_name);
        }

        // Safety check: the rename must not change resolution anywhere.
        let candidate = Analysis::new(new_text.clone());
        if resolution_shape(&candidate) != resolution_shape(self) {
            return Err(RenameError::Conflict);
        }

        let edits = spans.into_iter().map(|span| RenameEdit { span }).collect();
        Ok(Rename { edits, new_text })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renames_a_variable_and_all_uses() {
        let a = Analysis::new("let x = 1;\nlet y = x + x;\n");
        let pos = a.text().find('x').unwrap() as u32;
        let r = a.rename(pos, "total").unwrap();
        assert_eq!(r.edits.len(), 3);
        assert_eq!(r.new_text, "let total = 1;\nlet y = total + total;\n");
    }

    #[test]
    fn renames_from_a_use_site_too() {
        let a = Analysis::new("fn dbl(n) { n * 2 }\ndbl(21);\n");
        let pos = a.text().find("dbl(21)").unwrap() as u32;
        let r = a.rename(pos, "double").unwrap();
        assert_eq!(r.new_text, "fn double(n) { n * 2 }\ndouble(21);\n");
    }

    #[test]
    fn rejects_invalid_names() {
        let a = Analysis::new("let x = 1; x;");
        let pos = a.text().find('x').unwrap() as u32;
        assert_eq!(a.rename(pos, "1abc"), Err(RenameError::InvalidName));
        assert_eq!(a.rename(pos, "let"), Err(RenameError::InvalidName));
        assert_eq!(a.rename(pos, "a b"), Err(RenameError::InvalidName));
        assert_eq!(a.rename(pos, ""), Err(RenameError::InvalidName));
    }

    #[test]
    fn rejects_capturing_rename() {
        // Renaming the outer `a` to `b` would capture the use inside f, which
        // currently resolves to the parameter `b`.
        let a = Analysis::new("let a = 1; fn f(b) { a + b }");
        let pos = a.text().find("a =").unwrap() as u32;
        assert_eq!(a.rename(pos, "b"), Err(RenameError::Conflict));
    }

    #[test]
    fn unicode_rename_is_supported() {
        let a = Analysis::new("let café = 1; café + café;");
        let pos = a.text().find("café").unwrap() as u32;
        let r = a.rename(pos, "coffee").unwrap();
        assert_eq!(r.new_text, "let coffee = 1; coffee + coffee;");
    }

    #[test]
    fn nothing_to_rename_at_a_literal() {
        let a = Analysis::new("let x = 1;");
        // position on the integer literal, which is not a renameable symbol
        let pos = a.text().find('1').unwrap() as u32;
        assert_eq!(a.rename(pos, "y"), Err(RenameError::NotRenameable));
    }
}
