//! Incremental analysis.
//!
//! An [`Analysis`] holds the current source text broken into top level *units*,
//! one per top level statement. Each unit caches the tokens, the parsed
//! statement, and the parse diagnostics for its byte range. On an edit the
//! analyser reuses every unit before the edit untouched, reuses every unit after
//! the edit by shifting its spans, and re lexes and reparses only the affected
//! region in between. The symbol table is then rebuilt from the reassembled
//! statements.
//!
//! The guarantee, checked by the incremental gate, is that this produces tokens,
//! an AST, a symbol table, and diagnostics byte for byte identical to a full
//! from scratch analysis of the final text. Incremental work is an optimisation,
//! never a different answer.
//!
//! Reuse of the suffix after an edit is exact: when an edit deletes a separator
//! and merges the edited region into a following statement, the parser reports
//! [`crate::parser::ParseOutput::truncated`] and the region is extended to
//! absorb that statement before reparsing.

use crate::ast::*;
use crate::diagnostics::{self, Diagnostic};
use crate::lexer::{lex_at, Token};
use crate::parser::{parse_items, ItemParse};
use crate::resolver::{self, SymbolTable};

#[derive(Clone)]
struct Unit {
    lo: u32,
    hi: u32,
    stmt: Stmt,
    tokens: Vec<Token>,
    diags: Vec<Diagnostic>,
}

impl Unit {
    fn shifted(&self, delta: i64) -> Unit {
        Unit {
            lo: (self.lo as i64 + delta) as u32,
            hi: (self.hi as i64 + delta) as u32,
            stmt: shift_stmt(&self.stmt, delta),
            tokens: self.tokens.iter().map(|t| t.shifted(delta)).collect(),
            diags: self
                .diags
                .iter()
                .map(|d| Diagnostic {
                    span: d.span.shifted(delta),
                    ..d.clone()
                })
                .collect(),
        }
    }
}

/// A live, incrementally maintained analysis of one source file.
pub struct Analysis {
    text: String,
    units: Vec<Unit>,
    program: Program,
    tokens: Vec<Token>,
    symbols: SymbolTable,
    diagnostics: Vec<Diagnostic>,
}

impl Analysis {
    /// Analyse a source string from scratch.
    pub fn new(text: impl Into<String>) -> Analysis {
        let text = text.into();
        let tokens = lex_at(&text, 0);
        let (groups, _) = parse_items(&tokens);
        let units = build_units(groups, 0, text.len() as u32);
        let mut a = Analysis {
            text,
            units,
            program: Vec::new(),
            tokens: Vec::new(),
            symbols: SymbolTable::default(),
            diagnostics: Vec::new(),
        };
        a.rebuild();
        a
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn tokens(&self) -> &[Token] {
        &self.tokens
    }

    pub fn program(&self) -> &Program {
        &self.program
    }

    pub fn symbols(&self) -> &SymbolTable {
        &self.symbols
    }

    /// All diagnostics (parse and resolution) in a stable, sorted order.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Replace the byte range `[start, end)` with `replacement` and update the
    /// analysis incrementally.
    pub fn edit(&mut self, start: u32, end: u32, replacement: &str) {
        assert!(
            start <= end && end as usize <= self.text.len(),
            "edit out of bounds"
        );
        let (start_us, end_us) = (start as usize, end as usize);
        let delta = replacement.len() as i64 - (end - start) as i64;

        let mut new_text = String::with_capacity((self.text.len() as i64 + delta) as usize);
        new_text.push_str(&self.text[..start_us]);
        new_text.push_str(replacement);
        new_text.push_str(&self.text[end_us..]);

        // Partition existing units. Prefix ends before the edit; suffix begins at
        // or after the edit end; the rest is dirty and will be reparsed.
        let mut prefix: Vec<Unit> = Vec::new();
        let mut suffix: Vec<Unit> = Vec::new();
        for unit in std::mem::take(&mut self.units) {
            if unit.hi <= start {
                prefix.push(unit);
            } else if unit.lo >= end {
                suffix.push(unit);
            }
            // dirty units are dropped
        }
        suffix.sort_by_key(|u| u.lo);

        // Reparse the statement immediately before the edit as well. Its parse
        // can depend on the single token that follows it (a merged separator, an
        // inserted `;`), so it is never safe to reuse the unit ending right at
        // the edit.
        prefix.pop();

        // Align the left edge of the reparsed region to the start of a line. A
        // `//` comment runs to the end of its line, so a region that begins
        // mid line could inherit or shed comment state from a reused prefix
        // unit. Line aligned edges make re lexing independent of the reused
        // parts.
        let mut rs = prefix.last().map_or(0, |u| u.hi);
        while !prefix.is_empty() && !is_line_start(&new_text, rs) {
            prefix.pop();
            rs = prefix.last().map_or(0, |u| u.hi);
        }

        let text_len = new_text.len() as u32;

        // Candidate stop boundaries in new text coordinates.
        let boundaries: Vec<u32> = suffix
            .iter()
            .map(|u| (u.lo as i64 + delta) as u32)
            .chain(std::iter::once(text_len))
            .collect();

        let mut k = 0usize; // number of suffix units absorbed into the region
        let (region_units, stop) = loop {
            let stop = boundaries[k];
            let region_src = &new_text[rs as usize..stop as usize];
            let toks = lex_at(region_src, rs);
            let (groups, truncated) = parse_items(&toks);
            // Extend the region when the parse ran off the end, or when the right
            // edge is not a line start (same comment safety as the left edge).
            let needs_more = (truncated || !is_line_start(&new_text, stop)) && stop < text_len;
            if needs_more {
                k += 1;
                continue;
            }
            let units = build_units(groups, rs, stop);
            break (units, stop);
        };

        let mut new_units = prefix;
        new_units.extend(region_units);
        for unit in suffix.into_iter().skip(k) {
            debug_assert!((unit.lo as i64 + delta) as u32 >= stop);
            new_units.push(unit.shifted(delta));
        }

        self.text = new_text;
        self.units = new_units;
        self.rebuild();
    }

    fn rebuild(&mut self) {
        self.program = self.units.iter().map(|u| u.stmt.clone()).collect();
        self.tokens = self
            .units
            .iter()
            .flat_map(|u| u.tokens.iter().cloned())
            .collect();
        let parse_diags: Vec<Diagnostic> = self
            .units
            .iter()
            .flat_map(|u| u.diags.iter().cloned())
            .collect();
        self.symbols = resolver::resolve(&self.program);
        let mut all = parse_diags;
        all.extend(self.symbols.diagnostics.iter().cloned());
        self.diagnostics = diagnostics::sorted(all);
    }
}

fn is_line_start(text: &str, pos: u32) -> bool {
    pos == 0 || text.as_bytes().get(pos as usize - 1) == Some(&b'\n')
}

fn build_units(groups: Vec<ItemParse>, region_lo: u32, region_hi: u32) -> Vec<Unit> {
    let n = groups.len();
    if n == 0 {
        return Vec::new();
    }
    // Statement starts tile the region contiguously so an edit position always
    // maps into exactly one unit's [lo, hi) range.
    let starts: Vec<u32> = groups
        .iter()
        .enumerate()
        .map(|(i, g)| {
            if i == 0 {
                region_lo
            } else {
                g.tokens.first().map_or(region_lo, |t| t.span.start)
            }
        })
        .collect();

    groups
        .into_iter()
        .enumerate()
        .map(|(i, g)| {
            let lo = starts[i];
            let hi = if i + 1 < n { starts[i + 1] } else { region_hi };
            Unit {
                lo,
                hi,
                stmt: g.stmt,
                tokens: g.tokens,
                diags: g.diagnostics,
            }
        })
        .collect()
}

// Deep span shifting for reused suffix statements.

fn shift_ident(id: &Ident, delta: i64) -> Ident {
    Ident {
        name: id.name.clone(),
        span: id.span.shifted(delta),
    }
}

fn shift_stmt(stmt: &Stmt, delta: i64) -> Stmt {
    let kind = match &stmt.kind {
        StmtKind::Let(l) => StmtKind::Let(LetStmt {
            name: shift_ident(&l.name, delta),
            value: shift_expr(&l.value, delta),
        }),
        StmtKind::Fn(f) => StmtKind::Fn(FnDecl {
            name: shift_ident(&f.name, delta),
            params: f.params.iter().map(|p| shift_ident(p, delta)).collect(),
            body: shift_block(&f.body, delta),
        }),
        StmtKind::Expr(e) => StmtKind::Expr(shift_expr(e, delta)),
    };
    Stmt {
        kind,
        span: stmt.span.shifted(delta),
    }
}

fn shift_block(block: &Block, delta: i64) -> Block {
    Block {
        stmts: block.stmts.iter().map(|s| shift_stmt(s, delta)).collect(),
        tail: block.tail.as_ref().map(|e| Box::new(shift_expr(e, delta))),
        span: block.span.shifted(delta),
    }
}

fn shift_expr(expr: &Expr, delta: i64) -> Expr {
    let kind = match &expr.kind {
        ExprKind::Int(v) => ExprKind::Int(*v),
        ExprKind::Bool(v) => ExprKind::Bool(*v),
        ExprKind::Name(n) => ExprKind::Name(n.clone()),
        ExprKind::Error => ExprKind::Error,
        ExprKind::Unary { op, expr } => ExprKind::Unary {
            op: *op,
            expr: Box::new(shift_expr(expr, delta)),
        },
        ExprKind::Binary { op, lhs, rhs } => ExprKind::Binary {
            op: *op,
            lhs: Box::new(shift_expr(lhs, delta)),
            rhs: Box::new(shift_expr(rhs, delta)),
        },
        ExprKind::Call { callee, args } => ExprKind::Call {
            callee: Box::new(shift_expr(callee, delta)),
            args: args.iter().map(|a| shift_expr(a, delta)).collect(),
        },
        ExprKind::If {
            cond,
            then_block,
            else_block,
        } => ExprKind::If {
            cond: Box::new(shift_expr(cond, delta)),
            then_block: shift_block(then_block, delta),
            else_block: else_block.as_ref().map(|b| shift_block(b, delta)),
        },
        ExprKind::Block(b) => ExprKind::Block(shift_block(b, delta)),
        ExprKind::Paren(e) => ExprKind::Paren(Box::new(shift_expr(e, delta))),
    };
    Expr {
        kind,
        span: expr.span.shifted(delta),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(text: &str) -> (Vec<Token>, Program, Vec<Diagnostic>, SymbolTable) {
        let a = Analysis::new(text);
        (
            a.tokens().to_vec(),
            a.program().clone(),
            a.diagnostics().to_vec(),
            a.symbols().clone(),
        )
    }

    fn assert_matches_batch(a: &Analysis) {
        let b = Analysis::new(a.text());
        assert_eq!(a.tokens(), b.tokens(), "tokens differ for {:?}", a.text());
        assert_eq!(a.program(), b.program(), "ast differs for {:?}", a.text());
        assert_eq!(
            a.diagnostics(),
            b.diagnostics(),
            "diags differ for {:?}",
            a.text()
        );
        assert_eq!(
            a.symbols(),
            b.symbols(),
            "symbols differ for {:?}",
            a.text()
        );
    }

    #[test]
    fn edit_inside_one_statement() {
        let mut a = Analysis::new("let x = 1;\nlet y = 2;\n");
        // change the 2 to 42
        let pos = a.text().find('2').unwrap() as u32;
        a.edit(pos, pos + 1, "42");
        assert_eq!(a.text(), "let x = 1;\nlet y = 42;\n");
        assert_matches_batch(&a);
    }

    #[test]
    fn edit_deleting_separator_merges_region() {
        let mut a = Analysis::new("let x = 1;\nlet y = 2;\n");
        // delete the first semicolon, merging the two lets
        let pos = a.text().find(';').unwrap() as u32;
        a.edit(pos, pos + 1, "");
        assert_matches_batch(&a);
    }

    #[test]
    fn insert_new_statement_between() {
        let mut a = Analysis::new("let x = 1;\nlet y = x;\n");
        let pos = a.text().find("let y").unwrap() as u32;
        a.edit(pos, pos, "let z = x;\n");
        assert_matches_batch(&a);
    }

    #[test]
    fn incremental_equals_batch_after_many_edits() {
        let mut a = Analysis::new("fn main() { let a = 1; a }");
        a.edit(0, 0, "let g = 9;\n");
        a.edit(
            a.text().len() as u32,
            a.text().len() as u32,
            "\nlet h = g;\n",
        );
        let cut = a.text().find("a = 1").unwrap() as u32;
        a.edit(cut + 4, cut + 5, "100");
        assert_matches_batch(&a);
        let (_, prog, _, _) = batch(a.text());
        assert_eq!(a.program(), &prog);
    }
}
