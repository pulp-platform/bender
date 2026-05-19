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

#[test]
fn walk_design_extracts_modules_packages_and_instantiations() {
    let mut session = bender_slang::SlangSession::new();
    let files = vec![
        fixture_path("src/common_pkg.sv"),
        fixture_path("src/bus_intf.sv"),
        fixture_path("src/leaf.sv"),
        fixture_path("src/core.sv"),
        fixture_path("src/top.sv"),
    ];
    let includes = vec![fixture_path("include")];
    let defines = vec![];
    session
        .parse_group(&files, &includes, &defines)
        .expect("parse should succeed");

    let result = session.walk_design().expect("walk should succeed");

    // Expect one record per module/package/interface declaration.
    let by_name: std::collections::BTreeMap<_, _> =
        result.modules.iter().map(|m| (m.name.clone(), m)).collect();

    let common_pkg = by_name
        .get("common_pkg")
        .expect("common_pkg package should be present");
    assert!(
        common_pkg.is_package,
        "common_pkg should be flagged as a package"
    );

    let bus_intf = by_name
        .get("bus_intf")
        .expect("bus_intf interface should be present");
    assert!(
        bus_intf.is_interface,
        "bus_intf should be flagged as an interface"
    );

    let leaf = by_name.get("leaf").expect("leaf module should be present");
    assert!(!leaf.is_package && !leaf.is_interface);

    let core = by_name.get("core").expect("core module should be present");
    let core_params: Vec<&str> = core.parameters.iter().map(|p| p.name.as_str()).collect();
    assert!(
        core_params.contains(&"DefaultState"),
        "core should have DefaultState parameter, got {core_params:?}"
    );
    let core_insts: Vec<&str> = core
        .instantiations
        .iter()
        .map(|i| i.module_name.as_str())
        .collect();
    assert!(
        core_insts.contains(&"leaf"),
        "core should instantiate leaf, got {core_insts:?}"
    );

    let top = by_name.get("top").expect("top module should be present");
    let inst_modules: Vec<&str> = top
        .instantiations
        .iter()
        .map(|i| i.module_name.as_str())
        .collect();
    assert!(
        inst_modules.contains(&"core"),
        "top should instantiate core, got {inst_modules:?}"
    );
    assert!(
        inst_modules.contains(&"bus_intf"),
        "top should instantiate bus_intf, got {inst_modules:?}"
    );
    let imports: Vec<&str> = top
        .imports
        .iter()
        .map(|i| i.package_name.as_str())
        .collect();
    assert!(
        imports.contains(&"common_pkg"),
        "top should import common_pkg, got {imports:?}"
    );
}
