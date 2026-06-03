use std::path::PathBuf;

fn fixture_path(rel: &str) -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/pickle")
        .join(rel)
        .canonicalize()
        .expect("valid fixture path")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn parse_valid_file_succeeds() {
    let mut session = bender_slang::SlangSession::new();
    let files = vec![fixture_path("src/top.sv")];
    let includes = vec![fixture_path("include")];
    let defines = vec![];
    assert!(session.parse_group(&files, &includes, &defines).is_ok());
    assert_eq!(session.tree_count(), 1);
}

#[test]
fn parse_invalid_file_reported_via_parsed_ok() {
    // The contract: parse_group is lenient — system errors still throw, but per-file parse
    // errors are surfaced via ParsedTree::parsed_ok. Callers (bender script, bender pickle)
    // layer their own policy (allow / refuse / discriminate) on top.
    let mut session = bender_slang::SlangSession::new();
    let files = vec![fixture_path("src/broken.sv")];
    let includes = vec![];
    let defines = vec![];
    session
        .parse_group(&files, &includes, &defines)
        .expect("parse_group is lenient and should return Ok even on parse errors");

    let trees = session.all_trees();
    assert_eq!(trees.len(), 1);
    assert!(
        !trees[0].parsed_ok,
        "broken.sv should be reported as a failed parse"
    );
    assert!(
        !trees[0].encrypted,
        "broken.sv has no pragma protect envelope"
    );
}

#[test]
fn rewriter_build_from_trees_is_repeatable() {
    let mut session = bender_slang::SlangSession::new();
    let files = vec![fixture_path("src/top.sv")];
    let includes = vec![fixture_path("include")];
    let defines = vec![];
    session
        .parse_group(&files, &includes, &defines)
        .expect("parse should succeed");

    let trees = session.all_trees();
    let mut rewriter_once = bender_slang::SyntaxTreeRewriter::new();
    rewriter_once.set_prefix("p_");
    rewriter_once.set_suffix("_s");
    let first_pass_trees: Vec<_> = trees
        .iter()
        .map(|t| rewriter_once.rewrite_declarations(&t.tree))
        .collect();
    let renamed_once = rewriter_once.rewrite_references(
        first_pass_trees
            .first()
            .expect("one first-pass tree expected"),
    );
    assert!(
        renamed_once
            .display(bender_slang::SlangPrintOpts {
                expand_macros: false,
                include_directives: true,
                include_comments: true,
                squash_newlines: false,
            })
            .contains("module p_top_s (")
    );

    // Rebuilding with the same trees should remain stable.
    let mut rewriter_twice = bender_slang::SyntaxTreeRewriter::new();
    rewriter_twice.set_prefix("p_");
    rewriter_twice.set_suffix("_s");
    let first_pass_trees: Vec<_> = trees
        .iter()
        .map(|t| rewriter_twice.rewrite_declarations(&t.tree))
        .collect();
    let renamed_twice = rewriter_twice.rewrite_references(
        first_pass_trees
            .first()
            .expect("one first-pass tree expected"),
    );
    assert!(
        renamed_twice
            .display(bender_slang::SlangPrintOpts {
                expand_macros: false,
                include_directives: true,
                include_comments: true,
                squash_newlines: false,
            })
            .contains("module p_top_s (")
    );
}
