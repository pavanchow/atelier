# Atelier

Atelier is a dependency-free code-intelligence engine for a small language. It is
the semantic tooling layer an editor sits on top of. Give it source text and a
stream of edits and it gives you back tokens, a syntax tree, a scoped symbol
table, live diagnostics, and the queries an IDE needs: go to definition, find
references, hover, and a safe rename-symbol refactor.

It is written in pure Rust with the standard library only. No crates, no build
scripts, edition 2021.

Live playground: https://pavanchow.github.io/atelier/

## The gap it fills

Real language servers are large and pull in large dependency trees. That makes
them hard to read, hard to embed, and hard to trust as a teaching artifact. Yet
the ideas at the core of code intelligence are small and provable: lex, parse,
resolve names against lexical scopes, answer positional queries, and re-analyze
incrementally as text changes.

Atelier implements exactly that core, end to end, in code you can read in an
afternoon, with correctness gates that prove the interesting claims rather than
asserting them. It is useful in two situations.

- A person who wants to understand how go to definition, find references, and
  incremental analysis actually work, from tokens to queries, without wading
  through a production language server.
- An AI agent or tool that needs embeddable, auditable code intelligence for a
  small language with zero supply chain surface and behavior it can verify from
  the tests.

## Quickstart

```
cargo build --release
cargo test
./target/release/atelier demo
```

The CLI works on any file.

```
atelier analyze path/to/file        print diagnostics
atelier def     path/to/file 3:5    definition of the symbol at line 3, column 5
atelier refs    path/to/file 3:5    every reference to that symbol
atelier hover   path/to/file 3:5    symbol info at that position
atelier rename  path/to/file 3:5 n  rename the symbol at 3:5 to n, all sites
atelier run     path/to/file        evaluate the program
atelier demo                        a built-in tour of every feature
```

A position is a byte offset or a one-based `line:col`.

## The language

A compact expression and statement language with functions, let bindings,
blocks, calls, and control flow. It is small on purpose and still large enough
to have real lexical scopes and references.

```
fn fib(n) {
    if n < 2 { n } else { fib(n - 1) + fib(n - 2) }
}

let count = 10;
let result = fib(count);
result;
```

Functions are hoisted within their scope, so calls may precede declarations and
mutual recursion resolves. Parameters are visible throughout their body. A `let`
binding is visible after its declaration in the enclosing block. Blocks, function
bodies, and if branches each introduce a scope, and inner scopes may shadow outer
names.

Identifiers are unicode aware. A name may use any unicode letter, so `café` and
`число` are ordinary identifiers, and the lexer never splits a multi byte
character.

## API

```rust
use atelier::Analysis;

let mut a = Analysis::new("let x = 1;\nlet y = x + 1;\n");

// Queries take a byte offset.
let def = a.go_to_definition(15).unwrap();
let refs = a.find_references(15);
let info = a.hover(15);
let diags = a.diagnostics();

// Rename the symbol under the cursor everywhere it is used. The rename is
// refused if it would change name resolution, so it can never capture or
// dangle a name.
let renamed = a.rename(4, "value").unwrap();
assert_eq!(renamed.new_text, "let value = 1;\nlet y = value + 1;\n");

// Apply an edit as a byte range plus replacement text. The analysis updates
// incrementally and stays byte for byte identical to a full re-analysis.
a.edit(4, 5, "value");
```

A `Workspace` holds many files and runs one.

```rust
use atelier::Workspace;

let mut ws = Workspace::new();
ws.set_file("main.at", "fn dbl(n) { n * 2 }\ndbl(21);\n");
let output = ws.run_file("main.at").unwrap(); // prints 42
```

## The correctness gate

The three headline claims are proven by tests that check the engine against
independent oracles over many random programs. Sizes are bounded for CI and the
count is controllable with `ATELIER_FUZZ_OPS`.

1. Navigation correctness. Go to definition and find references from the
   production engine must agree with a separate, straightforward full-program
   scope walk on every symbol of every random program. Two independent resolvers
   agreeing is real cross-validation rather than a restatement.
2. Incremental equals batch. After a random sequence of edits applied
   incrementally, the tokens, syntax tree, symbol table, and diagnostics are
   identical to a full from-scratch analysis of the final text. Incremental work
   is an optimization and never a different answer.
3. Diagnostics and determinism. Diagnostics are correct, an in-scope name is
   never flagged and an out-of-scope name always is, and analyzing the same text
   twice yields identical output.
4. Rename correctness. Renaming to a fresh name always succeeds and leaves name
   resolution unchanged, and renaming to an existing name either preserves
   resolution or is refused. Both outcomes are cross-checked against the
   reference resolver, so an accepted rename provably resolves identically and a
   refused one provably would have changed resolution.
5. Boundary edits. Deleting a whole file, building one up from empty, commenting
   a statement out at a seam, fusing or splitting tokens, editing inside a deeply
   nested block, and editing a unicode identifier all keep incremental output
   identical to a full re-analysis.

The generator that feeds these gates is itself adversarial. It emits shadowing,
cross-scope references, redefinitions, hoisted forward references, nested blocks
and functions, and unicode identifiers, and the edit generator produces the seam
cases that break naive incremental engines.

Run them, and push the fuzzing harder if you like.

```
cargo test
ATELIER_FUZZ_OPS=2000 cargo test --release
```

## Design

See DESIGN.md for the architecture, the incremental analysis strategy, the
symbol table and resolution rules, and why each gate proves what it claims.

## License

MIT.
