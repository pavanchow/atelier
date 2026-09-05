//! The correctness gates. These are the load bearing tests: they assert the
//! headline claims of the engine against independent oracles over many random
//! programs. Sizes are bounded for CI and controllable via `ATELIER_FUZZ_OPS`.

use atelier::incremental::Analysis;
use atelier::reference;
use atelier::rng::Rng;
use atelier::span::Span;
use std::collections::BTreeMap;

fn fuzz_ops() -> u64 {
    std::env::var("ATELIER_FUZZ_OPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60)
}

/// The mapping the production engine believes: each identifier occurrence to the
/// declaration span it resolves to.
fn production_map(a: &Analysis) -> BTreeMap<Span, Option<Span>> {
    let sym = a.symbols();
    let mut map = BTreeMap::new();
    for occ in &sym.occurrences {
        let decl = occ.binding.map(|id| sym.bindings[id].decl_span);
        map.insert(occ.span, decl);
    }
    map
}

/// Gate 1: navigation correctness. For random valid programs, go to definition
/// and find references from the production engine must agree with an independent
/// full program scope walk on every symbol.
#[test]
fn gate_navigation_matches_reference_resolver() {
    let programs = fuzz_ops();
    for seed in 0..programs {
        let mut rng = Rng::new(seed.wrapping_mul(0x1000_0001));
        let src = atelier::gen::random_program(&mut rng, 14);
        let a = Analysis::new(&src);

        // Whole-program agreement: production resolution == reference resolution.
        let prod = production_map(&a);
        let refr = reference::resolve_program(a.program());
        assert_eq!(
            prod, refr,
            "resolution disagreement\nseed {seed}\nsrc:\n{src}"
        );

        // Per-occurrence query agreement.
        for occ in &a.symbols().occurrences {
            let pos = occ.span.start;
            let expected_decl = occ.binding.map(|id| a.symbols().bindings[id].decl_span);

            // go_to_definition
            let got = a.go_to_definition(pos).map(|d| d.span);
            assert_eq!(got, expected_decl, "go_to_definition seed {seed} src:\n{src}");

            // find_references must equal the set of occurrences sharing the binding.
            if let Some(id) = occ.binding {
                let expected: Vec<Span> = {
                    let mut v: Vec<Span> = a
                        .symbols()
                        .occurrences
                        .iter()
                        .filter(|o| o.binding == Some(id))
                        .map(|o| o.span)
                        .collect();
                    v.sort_by_key(|s| (s.start, s.end));
                    v
                };
                let got: Vec<Span> = a.find_references(pos).into_iter().map(|r| r.span).collect();
                assert_eq!(got, expected, "find_references seed {seed} src:\n{src}");
            }
        }
    }
}

/// Gate 2: incremental equals batch. After a random sequence of edits applied
/// incrementally, the tokens, AST, symbol table, and diagnostics must be
/// identical to a full from scratch analysis of the final text. This holds for
/// arbitrary edits, valid or not.
#[test]
fn gate_incremental_equals_batch() {
    let ops = fuzz_ops();
    for seed in 0..ops {
        let mut rng = Rng::new(seed.wrapping_mul(0x9E37_79B9).wrapping_add(7));
        let src = atelier::gen::random_program(&mut rng, 12);
        let mut a = Analysis::new(&src);

        let edits = 6 + rng.below(10);
        for step in 0..edits {
            let (start, end, repl) = atelier::gen::random_edit(&mut rng, a.text());
            a.edit(start, end, &repl);

            let batch = Analysis::new(a.text());
            assert_eq!(
                a.tokens(),
                batch.tokens(),
                "tokens diverged\nseed {seed} step {step}\ntext:\n{}",
                a.text()
            );
            assert_eq!(
                a.program(),
                batch.program(),
                "ast diverged\nseed {seed} step {step}\ntext:\n{}",
                a.text()
            );
            assert_eq!(
                a.symbols(),
                batch.symbols(),
                "symbols diverged\nseed {seed} step {step}\ntext:\n{}",
                a.text()
            );
            assert_eq!(
                a.diagnostics(),
                batch.diagnostics(),
                "diagnostics diverged\nseed {seed} step {step}\ntext:\n{}",
                a.text()
            );
        }
    }
}

/// Gate 3a: diagnostics are correct. A name in scope is never flagged, and a
/// name out of scope always is.
#[test]
fn gate_diagnostics_correct() {
    use atelier::DiagKind;

    let clean = Analysis::new("let a = 1; fn f(x) { a + x } f(a);");
    assert!(
        clean.diagnostics().is_empty(),
        "valid program flagged: {:?}",
        clean.diagnostics()
    );

    let unresolved = Analysis::new("let a = b;");
    assert!(unresolved
        .diagnostics()
        .iter()
        .any(|d| d.kind == DiagKind::UnresolvedName));

    let redef = Analysis::new("let a = 1; let a = 2;");
    assert!(redef
        .diagnostics()
        .iter()
        .any(|d| d.kind == DiagKind::Redefinition));

    let parse_err = Analysis::new("let a = ;");
    assert!(parse_err
        .diagnostics()
        .iter()
        .any(|d| d.kind == DiagKind::ParseError || d.kind == DiagKind::UnexpectedEof));
}

/// Gate 3b: determinism. Analysing the same text twice yields identical
/// diagnostics, tokens, AST, and symbols, over many random programs.
#[test]
fn gate_diagnostics_deterministic() {
    let programs = fuzz_ops();
    for seed in 0..programs {
        let mut rng = Rng::new(seed.wrapping_mul(31).wrapping_add(3));
        let src = atelier::gen::random_program(&mut rng, 14);
        let a = Analysis::new(&src);
        let b = Analysis::new(&src);
        assert_eq!(a.tokens(), b.tokens(), "tokens nondeterministic\n{src}");
        assert_eq!(a.program(), b.program(), "ast nondeterministic\n{src}");
        assert_eq!(a.diagnostics(), b.diagnostics(), "diags nondeterministic\n{src}");
        assert_eq!(a.symbols(), b.symbols(), "symbols nondeterministic\n{src}");
    }
}
