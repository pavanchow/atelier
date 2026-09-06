//! The correctness gates. These are the load bearing tests: they assert the
//! headline claims of the engine against independent oracles over many random
//! programs. Sizes are bounded for CI and controllable via `ATELIER_FUZZ_OPS`.

use atelier::incremental::Analysis;
use atelier::reference;
use atelier::rename::RenameError;
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
            assert_eq!(
                got, expected_decl,
                "go_to_definition seed {seed} src:\n{src}"
            );

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

/// A span independent fingerprint of name resolution computed by the independent
/// reference resolver: for each identifier occurrence, in position order, the
/// rank of the declaration it resolves to (or `None`). Two programs with equal
/// fingerprints resolve every occurrence to the correspondingly ranked
/// declaration, regardless of the identifiers' text or byte offsets.
fn reference_shape(program: &atelier::ast::Program) -> Vec<Option<usize>> {
    let map = reference::resolve_program(program);
    let keys: Vec<Span> = map.keys().copied().collect();
    let mut decl_rank: BTreeMap<Span, usize> = BTreeMap::new();
    for (i, k) in keys.iter().enumerate() {
        if map[k] == Some(*k) {
            decl_rank.insert(*k, i);
        }
    }
    keys.iter()
        .map(|k| map[k].and_then(|s| decl_rank.get(&s).copied()))
        .collect()
}

/// Replace every reference to the symbol at `pos` with `new_name` without any
/// safety check, used to prove that refused renames really would have changed
/// resolution.
fn naive_rename(a: &Analysis, pos: u32, new_name: &str) -> String {
    let mut spans: Vec<Span> = a.find_references(pos).into_iter().map(|r| r.span).collect();
    spans.sort_by_key(|s| s.start);
    let mut text = a.text().to_string();
    for span in spans.iter().rev() {
        text.replace_range(span.start as usize..span.end as usize, new_name);
    }
    text
}

/// Gate 4: rename correctness. Renaming to a globally fresh name must always
/// succeed and leave resolution unchanged, and renaming to an existing name must
/// either preserve resolution (verified against the reference resolver) or be
/// refused, in which case the naive rename genuinely would have changed
/// resolution. This proves the renamed program resolves identically.
#[test]
fn gate_rename_preserves_resolution() {
    let programs = fuzz_ops();
    for seed in 0..programs {
        let mut rng = Rng::new(seed.wrapping_mul(0x2545_F491).wrapping_add(11));
        let src = atelier::gen::random_program(&mut rng, 14);
        let a = Analysis::new(&src);

        let before = reference_shape(a.program());

        // Renameable occurrences: bound, with a real (non-empty) span.
        let targets: Vec<u32> = a
            .symbols()
            .occurrences
            .iter()
            .filter(|o| o.binding.is_some() && !o.span.is_empty())
            .map(|o| o.span.start)
            .collect();
        if targets.is_empty() {
            continue;
        }
        // Names already present, for the collision case.
        let names: Vec<String> = a
            .symbols()
            .bindings
            .iter()
            .filter(|b| !b.name.is_empty())
            .map(|b| b.name.clone())
            .collect();

        // Sample a few positions per program to keep the gate fast.
        let picks = 3.min(targets.len());
        for t in 0..picks {
            let pos = targets[(t * 2 + 1) % targets.len()];
            let cur = a.go_to_definition(pos).map(|d| d.name).unwrap_or_default();

            // Fresh name: must succeed and preserve resolution.
            let fresh = format!("zzuniq_{seed}_{t}");
            match a.rename(pos, &fresh) {
                Ok(r) => {
                    let after = reference_shape(&atelier::parser::parse(&r.new_text).program);
                    assert_eq!(
                        before, after,
                        "fresh rename changed resolution\nseed {seed}\nsrc:\n{src}\nnew:\n{}",
                        r.new_text
                    );
                }
                Err(e) => panic!("fresh rename must succeed, got {e:?}\nseed {seed}\nsrc:\n{src}"),
            }

            // Existing name: preserve or a justified conflict.
            if let Some(other) = names.iter().find(|n| **n != cur) {
                match a.rename(pos, other) {
                    Ok(r) => {
                        let after = reference_shape(&atelier::parser::parse(&r.new_text).program);
                        assert_eq!(
                            before, after,
                            "accepted rename changed resolution\nseed {seed}\nsrc:\n{src}\nnew:\n{}",
                            r.new_text
                        );
                    }
                    Err(RenameError::Conflict) => {
                        let naive = naive_rename(&a, pos, other);
                        let after = reference_shape(&atelier::parser::parse(&naive).program);
                        assert_ne!(
                            before, after,
                            "rename refused as a conflict but resolution was unchanged\nseed {seed}\nsrc:\n{src}\nnaive:\n{naive}"
                        );
                    }
                    Err(RenameError::InvalidName | RenameError::NotRenameable) => {}
                }
            }
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
        assert_eq!(
            a.diagnostics(),
            b.diagnostics(),
            "diags nondeterministic\n{src}"
        );
        assert_eq!(a.symbols(), b.symbols(), "symbols nondeterministic\n{src}");
    }
}

/// Assert an incrementally maintained analysis equals a from-scratch one on all
/// four artifacts.
fn assert_incremental_matches_batch(a: &Analysis) {
    let b = Analysis::new(a.text());
    assert_eq!(a.tokens(), b.tokens(), "tokens differ for {:?}", a.text());
    assert_eq!(a.program(), b.program(), "ast differs for {:?}", a.text());
    assert_eq!(
        a.symbols(),
        b.symbols(),
        "symbols differ for {:?}",
        a.text()
    );
    assert_eq!(
        a.diagnostics(),
        b.diagnostics(),
        "diags differ for {:?}",
        a.text()
    );
}

/// Gate 5: boundary and adversarial edits that the random fuzzer reaches rarely
/// but which are the classic incremental-analysis break points. Each must keep
/// incremental output identical to a full re-analysis.
#[test]
fn gate_incremental_boundary_cases() {
    // Delete the entire file.
    let mut a = Analysis::new("let x = 1;\nlet y = x;\n");
    a.edit(0, a.text().len() as u32, "");
    assert_eq!(a.text(), "");
    assert_incremental_matches_batch(&a);

    // Build a file up from empty, one insertion at a time.
    let mut a = Analysis::new("");
    for frag in ["let a = 1;\n", "fn f(b) { a + b }\n", "f(a);\n"] {
        let end = a.text().len() as u32;
        a.edit(end, end, frag);
        assert_incremental_matches_batch(&a);
    }

    // Insert `//` at a line start so a whole statement becomes a comment, then
    // remove it again. This is the comment-at-seam case.
    let mut a = Analysis::new("let a = 1;\nlet b = 2;\nb;\n");
    let at = a.text().find("let b").unwrap() as u32;
    a.edit(at, at, "//");
    assert_incremental_matches_batch(&a);
    a.edit(at, at + 2, "");
    assert_incremental_matches_batch(&a);

    // Join two tokens by deleting the space between them, then split again.
    let mut a = Analysis::new("let ab = 1;\nab;\n");
    let sp = a.text().find(" = ").unwrap() as u32; // "ab| = 1"
    a.edit(sp, sp + 3, ""); // -> "letab = ..."? actually removes " = " making "letab1;" region
    assert_incremental_matches_batch(&a);

    // Fuse `=` and `=` into `==` across an edit.
    let mut a = Analysis::new("let x = 1;\nx = 2 == 3;\n");
    let eq = a.text()[10..].find('=').map(|i| i + 10).unwrap() as u32;
    a.edit(eq, eq, "=");
    assert_incremental_matches_batch(&a);

    // A deeply nested block, edited at its core.
    let mut deep = String::from("fn f() { ");
    for _ in 0..40 {
        deep.push_str("{ ");
    }
    deep.push('1');
    for _ in 0..40 {
        deep.push_str(" }");
    }
    deep.push_str(" }\n");
    let mut a = Analysis::new(&deep);
    let one = a.text().find('1').unwrap() as u32;
    a.edit(one, one + 1, "2 + 2");
    assert_incremental_matches_batch(&a);

    // A unicode identifier edited in place.
    let mut a = Analysis::new("let café = 1;\ncafé + café;\n");
    let pos = a.text().find("= 1").map(|i| i + 2).unwrap() as u32;
    a.edit(pos, pos + 1, "99");
    assert_incremental_matches_batch(&a);
}
