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
fn parse_invalid_file_returns_parse_error() {
    let mut session = bender_slang::SlangSession::new();
    let files = vec![fixture_path("src/broken.sv")];
    let includes = vec![];
    let defines = vec![];
    let result = session.parse_group(&files, &includes, &defines);

    match result {
        Err(bender_slang::SlangError::ParseGroup { .. }) => {}
        Err(other) => panic!("expected SlangError::ParseGroup, got {other}"),
        Ok(_) => panic!("expected parse to fail"),
    }
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

    let trees = session.all_trees().expect("tree collection should succeed");
    let mut rewriter_once = bender_slang::SyntaxTreeRewriter::new();
    rewriter_once.set_prefix("p_");
    rewriter_once.set_suffix("_s");
    let first_pass_trees: Vec<_> = trees
        .iter()
        .map(|t| rewriter_once.rewrite_declarations(t))
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
        .map(|t| rewriter_twice.rewrite_declarations(t))
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
