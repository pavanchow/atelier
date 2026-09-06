//! Random program and edit generators for the correctness gates.
//!
//! [`random_program`] emits structurally valid programs. It deliberately
//! exercises the cases that make name resolution and incremental analysis hard:
//! shadowing across nested scopes, references to outer bindings, redefinitions in
//! one scope, hoisted forward references to functions, deeply nested blocks and
//! functions, and unicode identifiers. [`random_edit`] produces arbitrary text
//! splices, valid or not, including edits that split or join tokens and comments
//! at their seams, so the incremental equals batch invariant is stressed against
//! the messiest input an editor could send.

use crate::rng::Rng;

struct Builder<'r> {
    rng: &'r mut Rng,
    out: String,
    counter: usize,
    budget: i32,
}

/// Identifier stems mixed into the fresh-name pool so the lexer, resolver, and
/// span arithmetic all meet multi byte identifiers. Each is a valid identifier
/// start under the unicode aware lexer.
const NAME_STEMS: &[&str] = &["v", "café", "λ", "число", "名前", "naïve", "Ω"];

/// Generate a random valid program. `size` bounds how much is emitted.
pub fn random_program(rng: &mut Rng, size: u32) -> String {
    let mut b = Builder {
        rng,
        out: String::new(),
        counter: 0,
        budget: size as i32,
    };
    let mut visible: Vec<String> = Vec::new();
    let mut fns: Vec<String> = Vec::new();
    let items = 1 + b.rng.below(size as usize + 1);
    for _ in 0..items {
        if b.budget <= 0 {
            break;
        }
        b.emit_item(&mut visible, 0, &mut fns, 0);
        b.out.push('\n');
    }
    b.out
}

impl Builder<'_> {
    /// A fresh, unique identifier. Sometimes drawn from a unicode stem so the
    /// pipeline is exercised on multi byte names. The counter suffix keeps it
    /// unique unless a caller deliberately reuses a name.
    fn fresh(&mut self) -> String {
        self.counter += 1;
        let stem = self.rng.pick(NAME_STEMS);
        format!("{stem}{}", self.counter)
    }

    /// Pick the name for a new binding. Mostly fresh, but sometimes reuses a name
    /// already bound in this scope (a redefinition) or one from an enclosing
    /// scope (shadowing), which are the cases resolution has to get right.
    fn decl_name(&mut self, visible: &[String], scope_start: usize) -> String {
        let roll = self.rng.below(100);
        if roll < 12 && scope_start < visible.len() {
            // redefinition: reuse a name declared in this same scope
            visible[scope_start + self.rng.below(visible.len() - scope_start)].clone()
        } else if roll < 26 && scope_start > 0 {
            // shadowing: reuse a name from an enclosing scope
            visible[self.rng.below(scope_start)].clone()
        } else {
            self.fresh()
        }
    }

    fn emit_item(
        &mut self,
        visible: &mut Vec<String>,
        scope_start: usize,
        fns: &mut Vec<String>,
        depth: u32,
    ) {
        self.budget -= 1;
        let can_nest = depth < 3;
        let choice = self.rng.below(3);
        match choice {
            0 => {
                let name = self.decl_name(visible, scope_start);
                self.out.push_str("let ");
                self.out.push_str(&name);
                self.out.push_str(" = ");
                self.emit_expr(visible, fns, depth);
                self.out.push_str("; ");
                visible.push(name);
            }
            1 if can_nest => {
                let name = self.fresh();
                let arity = self.rng.below(3);
                self.out.push_str("fn ");
                self.out.push_str(&name);
                self.out.push('(');
                let inner_start = visible.len();
                for i in 0..arity {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    // params may shadow an enclosing name, or (rarely) collide
                    let pname = if self.rng.chance(1, 6) && inner_start > 0 {
                        visible[self.rng.below(inner_start)].clone()
                    } else {
                        self.fresh()
                    };
                    self.out.push_str(&pname);
                    visible.push(pname);
                }
                self.out.push_str(") { ");
                // Register the function name so its body and later siblings can
                // call it (hoisting and forward references).
                fns.push(name);
                let mut inner_fns = fns.clone();
                let stmts = self.rng.below(3);
                for _ in 0..stmts {
                    if self.budget <= 0 {
                        break;
                    }
                    self.emit_item(visible, inner_start, &mut inner_fns, depth + 1);
                }
                self.emit_expr(visible, &inner_fns, depth + 1);
                self.out.push_str(" } ");
                visible.truncate(inner_start);
            }
            _ => {
                self.emit_expr(visible, fns, depth);
                self.out.push_str("; ");
            }
        }
    }

    fn emit_expr(&mut self, visible: &mut Vec<String>, fns: &[String], depth: u32) {
        self.budget -= 1;
        let leaf = depth >= 4 || self.budget <= 0 || self.rng.chance(1, 2);
        if leaf {
            match self.rng.below(3) {
                0 if !visible.is_empty() => {
                    let n = self.rng.pick(visible).clone();
                    self.out.push_str(&n);
                }
                1 => self.out.push_str(if self.rng.chance(1, 2) {
                    "true"
                } else {
                    "false"
                }),
                _ => {
                    let n = self.rng.below(100);
                    self.out.push_str(&n.to_string());
                }
            }
            return;
        }
        match self.rng.below(5) {
            0 => {
                self.emit_expr(visible, fns, depth + 1);
                let op = self.rng.pick(&["+", "-", "*", "<", "==", ">"]);
                self.out.push(' ');
                self.out.push_str(op);
                self.out.push(' ');
                self.emit_expr(visible, fns, depth + 1);
            }
            1 if !fns.is_empty() => {
                let f = self.rng.pick(fns).clone();
                self.out.push_str(&f);
                self.out.push('(');
                let args = self.rng.below(3);
                for i in 0..args {
                    if i > 0 {
                        self.out.push_str(", ");
                    }
                    self.emit_expr(visible, fns, depth + 1);
                }
                self.out.push(')');
            }
            2 => {
                self.out.push('(');
                self.emit_expr(visible, fns, depth + 1);
                self.out.push(')');
            }
            3 => {
                // A block expression introduces its own scope, so an inner `let`
                // can shadow an outer name and go out of scope at the closing brace.
                self.out.push_str("{ ");
                let inner_start = visible.len();
                let stmts = self.rng.below(2);
                for _ in 0..stmts {
                    if self.budget <= 0 {
                        break;
                    }
                    let name = self.decl_name(visible, inner_start);
                    self.out.push_str("let ");
                    self.out.push_str(&name);
                    self.out.push_str(" = ");
                    self.emit_expr(visible, fns, depth + 1);
                    self.out.push_str("; ");
                    visible.push(name);
                }
                self.emit_expr(visible, fns, depth + 1);
                self.out.push_str(" }");
                visible.truncate(inner_start);
            }
            _ => {
                self.out.push_str("if ");
                self.emit_expr(visible, fns, depth + 1);
                self.out.push_str(" { ");
                self.emit_expr(visible, fns, depth + 1);
                self.out.push_str(" } else { ");
                self.emit_expr(visible, fns, depth + 1);
                self.out.push_str(" }");
            }
        }
    }
}

/// Splice fragments for [`random_edit`]. The set is chosen to force hard seam
/// cases: bare operators that can fuse with a neighbour (`=` next to `=` makes
/// `==`), digits and letters that extend an adjacent token, comment starts with
/// and without a trailing newline, unbalanced braces, unicode identifiers and
/// symbols, and newlines that move line boundaries.
const SNIPPETS: &[&str] = &[
    "",
    "x",
    ";",
    " ",
    "1",
    "42",
    "let q = 1;",
    "fn t() { 0 }",
    "(",
    ")",
    "{",
    "}",
    "+",
    "=",
    "*",
    "if a { 1 } else { 2 }",
    "\n",
    "foo(",
    "* 3",
    "true",
    "// c\n",
    "//tail",
    "v1",
    "9",
    "café",
    "λ",
    "€",
    "let café = λ;",
    "// α\n",
    "==",
    "\n// x",
    "}\n",
    "b b",
];

/// Produce a random edit `(start, end, replacement)` for the given text.
pub fn random_edit(rng: &mut Rng, text: &str) -> (u32, u32, String) {
    let len = text.len();
    let a = rng.below(len + 1);
    let b = rng.below(len + 1);
    let (mut start, mut end) = (a.min(b), a.max(b));
    // keep edits reasonably local
    if end - start > 12 {
        end = start + 12;
    }
    start = floor_char_boundary(text, start);
    end = floor_char_boundary(text, end);
    let repl = rng.pick(SNIPPETS).to_string();
    (start as u32, end as u32, repl)
}

fn floor_char_boundary(text: &str, mut i: usize) -> usize {
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i.min(text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse;

    #[test]
    fn generated_programs_mostly_parse_clean() {
        let mut clean = 0;
        for seed in 0..200u64 {
            let mut rng = Rng::new(seed);
            let src = random_program(&mut rng, 12);
            if parse(&src).diagnostics.is_empty() {
                clean += 1;
            }
        }
        // The generator targets valid syntax; the vast majority should be clean.
        assert!(clean > 180, "only {clean}/200 parsed clean");
    }

    #[test]
    fn generator_exercises_shadowing_and_unicode() {
        // Over a spread of seeds the generator should produce redefinition or
        // shadowing (a name bound twice) and multi byte identifiers, otherwise
        // the hard cases are not actually being covered.
        let mut saw_unicode = false;
        let mut saw_repeat_binding = false;
        for seed in 0..300u64 {
            let mut rng = Rng::new(seed.wrapping_mul(2_654_435_761));
            let src = random_program(&mut rng, 16);
            if !src.is_ascii() {
                saw_unicode = true;
            }
            for stem in ["v", "café"] {
                let decl = format!("let {stem}");
                if src.matches(&decl).count() >= 2 {
                    saw_repeat_binding = true;
                }
            }
            if saw_unicode && saw_repeat_binding {
                break;
            }
        }
        assert!(saw_unicode, "generator never emitted a unicode identifier");
        assert!(
            saw_repeat_binding,
            "generator never emitted a repeated binding"
        );
    }
}
