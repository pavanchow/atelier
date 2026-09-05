//! Random program and edit generators for the correctness gates.
//!
//! [`random_program`] emits structurally valid programs that only reference
//! names already in scope, so they resolve cleanly. [`random_edit`] produces
//! arbitrary text splices, valid or not, to stress the incremental analyser: the
//! incremental equals batch invariant must hold for any edit.

use crate::rng::Rng;

struct Builder<'r> {
    rng: &'r mut Rng,
    out: String,
    counter: usize,
    budget: i32,
}

/// Generate a random valid program. `size` bounds how much is emitted.
pub fn random_program(rng: &mut Rng, size: u32) -> String {
    let mut b = Builder {
        rng,
        out: String::new(),
        counter: 0,
        budget: size as i32,
    };
    let mut names: Vec<String> = Vec::new();
    let mut fns: Vec<String> = Vec::new();
    let items = 1 + b.rng.below(size as usize + 1);
    for _ in 0..items {
        if b.budget <= 0 {
            break;
        }
        b.emit_item(&mut names, &mut fns, 0);
        b.out.push('\n');
    }
    b.out
}

impl<'r> Builder<'r> {
    fn fresh(&mut self) -> String {
        self.counter += 1;
        format!("v{}", self.counter)
    }

    fn emit_item(&mut self, names: &mut Vec<String>, fns: &mut Vec<String>, depth: u32) {
        self.budget -= 1;
        match self.rng.below(3) {
            0 => {
                // let binding
                let name = self.fresh();
                self.out.push_str("let ");
                self.out.push_str(&name);
                self.out.push_str(" = ");
                self.emit_expr(names, fns, depth);
                self.out.push_str("; ");
                names.push(name);
            }
            1 if depth < 2 => {
                // function declaration
                let name = self.fresh();
                let arity = self.rng.below(3);
                let mut params = Vec::new();
                for _ in 0..arity {
                    params.push(self.fresh());
                }
                self.out.push_str("fn ");
                self.out.push_str(&name);
                self.out.push('(');
                self.out.push_str(&params.join(", "));
                self.out.push_str(") { ");
                fns.push(name.clone());
                let mut inner_names = params.clone();
                let mut inner_fns = fns.clone();
                let stmts = self.rng.below(3);
                for _ in 0..stmts {
                    if self.budget <= 0 {
                        break;
                    }
                    self.emit_item(&mut inner_names, &mut inner_fns, depth + 1);
                }
                // tail expression
                self.emit_expr(&inner_names, &inner_fns, depth + 1);
                self.out.push_str(" } ");
            }
            _ => {
                // expression statement
                self.emit_expr(names, fns, depth);
                self.out.push_str("; ");
            }
        }
    }

    fn emit_expr(&mut self, names: &[String], fns: &[String], depth: u32) {
        self.budget -= 1;
        let leaf = depth >= 3 || self.budget <= 0 || self.rng.chance(1, 2);
        if leaf {
            match self.rng.below(3) {
                0 if !names.is_empty() => {
                    let n = self.rng.pick(names).clone();
                    self.out.push_str(&n);
                }
                1 => self.out.push_str(if self.rng.chance(1, 2) { "true" } else { "false" }),
                _ => {
                    let n = self.rng.below(100);
                    self.out.push_str(&n.to_string());
                }
            }
            return;
        }
        match self.rng.below(4) {
            0 => {
                self.emit_expr(names, fns, depth + 1);
                let op = self.rng.pick(&["+", "-", "*", "<", "==", ">"]);
                self.out.push(' ');
                self.out.push_str(op);
                self.out.push(' ');
                self.emit_expr(names, fns, depth + 1);
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
                    self.emit_expr(names, fns, depth + 1);
                }
                self.out.push(')');
            }
            2 => {
                self.out.push('(');
                self.emit_expr(names, fns, depth + 1);
                self.out.push(')');
            }
            _ => {
                self.out.push_str("if ");
                self.emit_expr(names, fns, depth + 1);
                self.out.push_str(" { ");
                self.emit_expr(names, fns, depth + 1);
                self.out.push_str(" } else { ");
                self.emit_expr(names, fns, depth + 1);
                self.out.push_str(" }");
            }
        }
    }
}

const SNIPPETS: &[&str] = &[
    "", "x", ";", " ", "1", "42", "let q = 1;", "fn t() { 0 }", "(", ")", "{", "}", "+",
    "if a { 1 } else { 2 }", "\n", "foo(", "* 3", "true", "// c\n", "v1",
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
}
