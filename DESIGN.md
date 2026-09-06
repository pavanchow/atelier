# Design

Atelier is a pipeline. Text becomes tokens, tokens become a syntax tree, the
tree becomes a scoped symbol table, and the symbol table answers IDE queries. An
incremental layer wraps the pipeline so an edit updates only what changed. This
document explains each stage, the incremental strategy, the resolution rules, and
why each correctness gate proves its claim.

## Architecture

The crate is a small set of focused modules.

- `span` holds byte-offset source spans and line and column conversion.
- `lexer` turns text into tokens with absolute spans. It skips whitespace and
  line comments and emits a single-character error token for anything it does not
  recognize.
- `ast` defines the tree. Every node carries its span, and nodes compare by value
  including their spans.
- `parser` is a recursive descent parser with error recovery. It parses a token
  slice into per-statement groups so the incremental layer can hand it just the
  tokens around an edit.
- `resolver` builds the symbol table by walking the tree with a scope stack. It
  records every identifier occurrence together with the binding it resolves to.
- `reference` is a second, independent resolver used only by the tests. It is a
  deliberately plain full-program scope walk that maps each occurrence to a
  declaration. It shares the tree but none of the production machinery.
- `incremental` is the heart of the project. It maintains an `Analysis`, applies
  edits, reuses unedited work, and rebuilds the symbol table.
- `query` answers go to definition, find references, and hover.
- `rename` is the rename-symbol refactor. It rewrites a binding and every
  reference and refuses any rename that would change name resolution.
- `eval` is a tree-walking evaluator so a program can Run.
- `workspace` is the multi-file project model and the run task.
- `rng` and `gen` are the seedable generator that feeds the gates.

## The language

The language is an expression and statement language. A program is a sequence of
statements. A statement is a `let` binding, a function declaration, or an
expression followed by a semicolon. A block is a brace-delimited scope that holds
statements and an optional final expression that gives the block its value.
Expressions cover integer and boolean literals, names, unary and binary
operators with the usual precedence, calls, parenthesized groups, blocks, and
if with an optional else.

It is small on purpose. It is also large enough to have functions, parameters,
nested blocks, shadowing, and both forward and backward references, which is
everything needed to make name resolution and navigation nontrivial.

## The lexer

The lexer is a single left-to-right pass. Identifiers and keywords share a scan,
numbers are digit runs, and operators handle the two-character forms before the
one-character forms. The only state that reaches beyond a single character is the
line comment, which runs from a pair of slashes to the end of the line. That one
fact drives a decision in the incremental layer, described below.

The lexer is unicode aware. An identifier may start with any unicode letter or an
underscore and continue with any letter or digit, so `café` and `число` are
identifiers. The scan advances one whole character at a time, and an unrecognized
character becomes a single-character error token covering the whole codepoint, so
the lexer never slices a multi byte character and never panics on arbitrary text.
Because every offset is still a byte offset, spans stay exact for multi byte
input and the incremental layer needs no special case for it.

## The parser and error recovery

Parsing is layered recursive descent, one function per precedence level. Every
path is guaranteed to make progress, so no input can hang the parser.

Recovery matters because an editor spends most of its time looking at code that
does not yet parse. When a statement is malformed the parser records a diagnostic
and skips to the next statement boundary, a semicolon or a keyword that starts a
new statement or a closing brace. Missing terminators produce a diagnostic anchored
to the end of the construct that was parsed.

The parser exposes its output grouped by statement. Each group carries the
statement, exactly the tokens it consumed, and exactly the diagnostics its parse
produced. Grouping by production rather than by span is important. A missing-
semicolon diagnostic points at the following token, which belongs to the next
statement, yet it was produced by this one. Grouping by production keeps it with
the statement that owns it, which is what makes reuse safe.

## Incremental analysis

An `Analysis` stores the source split into units, one unit per top-level
statement. Each unit caches its byte range, its parsed statement, its tokens, and
its parse diagnostics. On an edit the analysis does three things.

First it partitions the existing units. Units that end before the edit are the
prefix and are reused untouched. Units that begin at or after the edit end are the
suffix and are reused by shifting their spans by the edit delta. Everything that
overlaps the edit is discarded.

Second it reparses a region. The region runs from the end of the reused prefix to
the start of the reused suffix. The analysis re-lexes only that region and
reparses only the statements inside it. The reused prefix and suffix pay no lexing
or parsing cost at all, only a span shift for the suffix.

Third it rebuilds the symbol table from the reassembled statements and merges the
parse diagnostics with the resolution diagnostics.

Three details make this exact rather than approximate.

- The statement immediately before the edit is always reparsed. Its parse can
  depend on the single token that follows it, for example an inserted semicolon
  that merges it with the edit, so it is never reused.
- The region is extended when the parse runs off its end. The parser reports
  truncation when the token stream ends inside a construct, and recovery reports
  it when a skip reaches the end without finding a boundary. On truncation the
  region absorbs the next suffix unit and reparses. This is how a deleted
  separator that merges two statements is handled.
- The region edges are aligned to line starts. A line comment runs to the end of
  its line, so a region that began or ended in the middle of a line could inherit
  or shed comment state from a reused unit. Because comment state resets at a
  newline, snapping both edges to line starts makes re-lexing independent of the
  reused parts.

The result is that the incremental output is identical to a full re-analysis, not
merely close. That is the property the second gate checks.

## The symbol table and resolution

Resolution is a walk with a scope stack. The rules are these.

- Functions are hoisted within their enclosing scope. A pre-pass registers every
  function declaration in a scope before the scope is walked, so a call may
  appear before the declaration and mutual recursion resolves.
- Parameters seed the function body scope and are visible throughout it.
- A `let` binding becomes visible only after its declaration, sequentially,
  within the enclosing block.
- Blocks, function bodies, and both if branches each introduce a scope. An inner
  scope may shadow an outer name. Declaring the same name twice in one scope is a
  redefinition diagnostic on the second declaration.
- A name resolves to the nearest enclosing scope that binds it. Within a scope the
  sequential bindings visible so far are checked, then the hoisted functions.

The resolver records an occurrence for every declaration and every use, each
tagged with the binding it resolves to. Go to definition follows an occurrence to
its binding. Find references gathers every occurrence of that binding. Hover reads
the binding metadata. Because every query reads the same recorded occurrences, the
three queries cannot disagree with each other.

## Evaluation

The evaluator is a straightforward tree walk. Values are integers, booleans, and
unit. Let bindings and blocks follow the sequential scope rules, if yields the
value of the taken branch, and arithmetic is checked so overflow and division by
zero become runtime errors rather than panics. Top-level functions are hoisted so
they can recurse and call one another. Running a program prints the value of each
top-level expression statement.

## Rename

Rename is the one refactor that has to be exactly right, because it edits the
user's code. Atelier makes it safe by construction. To rename the symbol at a
position, the renamer gathers every occurrence of that binding through find
references, rewrites each occurrence right to left so earlier offsets stay valid,
and produces candidate text. It then re-analyses the candidate and compares a
fingerprint of name resolution before and after. The fingerprint records, for
each identifier occurrence in position order, the rank of the declaration it
resolves to. If the fingerprints differ the rename would have changed which
declaration some name binds to, which is capture or a collision, so it is refused
and the original text is left untouched. A new name is also validated as a single
identifier first, so a rename can never introduce a syntax error.

The consequence is a strong guarantee. An accepted rename produces a program that
resolves every name exactly as the original did, and an unsafe rename is reported
rather than applied. Renaming to a name that appears nowhere in the file is always
safe and always accepted.

## Why each gate proves its claim

The gates live in the tests and run over many seeded random programs.

The navigation gate builds the production symbol table and compares its
occurrence-to-declaration mapping against the independent reference resolver, then
checks go to definition and find references at every occurrence. The two resolvers
are separate implementations of the same rules. When they agree on every symbol of
every random program, the navigation answers are correct rather than merely
self-consistent. This is the point of writing the second resolver at all. A query
checked only against the machinery that produced it proves nothing.

The incremental gate starts from a random program, applies a random sequence of
edits incrementally, and after each edit compares the tokens, the tree, the symbol
table, and the diagnostics against a full from-scratch analysis of the current
text. The edits are arbitrary splices, valid or not, because the invariant must
hold for any edit an editor could send. Equality across all four artifacts is the
strongest possible statement that incremental analysis is an optimization and
never a different answer. The tree comparison includes spans, so even a
mispositioned node fails the gate.

The diagnostics gate checks that a name in scope is never flagged and a name out
of scope always is, that redefinitions and parse errors are reported, and that
analyzing the same text twice is bit-for-bit identical. Correctness and
determinism together are what a tool downstream depends on.

The rename gate proves the refactor is semantics preserving. For each random
program it renames sampled symbols in two ways. Renaming to a globally fresh name
must always succeed, and the renamed program, resolved by the independent
reference resolver, must have the same resolution fingerprint as the original.
Renaming to an existing name must either preserve that fingerprint or be refused,
and when it is refused the gate performs the rename without the safety check and
confirms that the fingerprint really would have changed. Checking with the
reference resolver rather than the production one keeps the proof honest, since
the renamer's own safety check uses the production resolver.

The boundary gate pins down the classic incremental break points that random
edits reach only rarely: deleting the whole file, growing it from empty, turning
a statement into a comment at a line seam, fusing and splitting tokens, editing
inside a deeply nested block, and editing a unicode identifier. Each must keep
incremental output identical to a full re-analysis.

The generators are adversarial on purpose. Programs carry shadowing, cross-scope
references, redefinitions, hoisted forward references, nested blocks and
functions, and unicode identifiers, and the edit generator emits the token and
comment seam cases that a naive incremental engine gets wrong.

All of them are bounded for continuous integration and scale up through the
`ATELIER_FUZZ_OPS` environment variable, so the same tests double as a fuzzer.
