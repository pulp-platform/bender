// Copyright (c) 2026 ETH Zurich
// Alessandro Ottaviano <aottaviano@tenstorrent.com>

//! End-to-end test driving the full kg pipeline (extract -> store -> search)
//! against the existing bender pickle fixtures. Verifies that the parsed
//! design produces the expected module / instantiation / import topology.

use std::path::PathBuf;

use bender_kg_core::{CoreConfig, Engine};
use bender_kg_extract::{ExtractInputs, SourceGroupInput};

fn pickle_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/pickle")
        .canonicalize()
        .expect("pickle fixture should exist")
}

fn fixture_file(rel: &str) -> String {
    pickle_root().join(rel).to_string_lossy().into_owned()
}

fn fixture_inputs(workspace: &str) -> ExtractInputs {
    ExtractInputs {
        workspace: workspace.to_string(),
        targets: vec!["rtl".into()],
        tops: vec!["top".into()],
        // Default fixture turns elab ON so tests can assert against
        // `resolved_param_values` / `resolved_port_widths`. Tests that
        // exercise the no-elab path flip this off explicitly.
        elab: true,
        design_alias: Some("pickle".into()),
        groups: vec![SourceGroupInput {
            files: vec![
                fixture_file("src/common_pkg.sv"),
                fixture_file("src/bus_intf.sv"),
                fixture_file("src/leaf.sv"),
                fixture_file("src/core.sv"),
                fixture_file("src/top.sv"),
            ],
            include_dirs: vec![pickle_root().join("include").to_string_lossy().into_owned()],
            defines: vec![],
        }],
        ..Default::default()
    }
}

#[tokio::test(flavor = "current_thread")]
async fn build_extracts_modules_and_hierarchy_from_pickle_fixture() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = CoreConfig::new(tmp.path());
    cfg.skip_embeddings = false;
    let mut engine = Engine::open(cfg).await.expect("open engine");

    let inputs = fixture_inputs(&pickle_root().to_string_lossy());
    let outcome = engine.build(&inputs).await.expect("build should succeed");

    // Five top-level declarations: top, core, leaf, bus_intf, common_pkg.
    assert!(
        outcome.modules_indexed >= 5,
        "expected at least 5 modules, got {}",
        outcome.modules_indexed
    );
    assert!(outcome.embeddings_indexed > 0);

    // Module retrieval --------------------------------------------------------
    let top = engine
        .get_module("top")
        .unwrap()
        .expect("top module should be present");
    assert!(!top.is_package);
    assert_eq!(top.design, outcome.manifest.identity.alias);

    let core = engine
        .get_module("core")
        .unwrap()
        .expect("core module should be present");
    assert!(!core.is_package);
    assert!(
        core.parameters.iter().any(|p| p.name == "DefaultState"),
        "core should expose DefaultState parameter, got {:?}",
        core.parameters
    );

    let common_pkg = engine
        .get_module("common_pkg")
        .unwrap()
        .expect("common_pkg should be present");
    assert!(common_pkg.is_package);

    // Subgraph traversal ------------------------------------------------------
    let sub = engine.get_subgraph("top", 2).expect("subgraph");
    let modules: Vec<&str> = sub.nodes.iter().map(|m| m.name.as_str()).collect();
    assert!(modules.contains(&"top"));
    assert!(modules.contains(&"core"));
    assert!(
        modules.contains(&"leaf"),
        "expected leaf reachable at depth=2, got {modules:?}"
    );

    // Hierarchy path ----------------------------------------------------------
    let path = engine
        .trace_hierarchy_path("top", "leaf")
        .expect("hierarchy path");
    assert!(!path.is_empty(), "expected a top -> leaf path");

    // Elaborated parameter forwarding ----------------------------------------
    // top instantiates core with .DefaultState(common_pkg::Error). The walk
    // captures the textual expression in `param_bindings`; elab folds it to
    // a literal in `resolved_param_values`. Both must be present.
    let top_to_core = engine
        .get_instance_context("top", "core")
        .expect("top->core context");
    assert_eq!(top_to_core.len(), 1);
    let edge = &top_to_core[0];
    assert_eq!(
        edge.param_bindings.get("DefaultState").map(String::as_str),
        Some("common_pkg::Error"),
        "textual call-site expression should survive elab",
    );
    assert!(
        edge.resolved_param_values
            .get("DefaultState")
            .is_some_and(|v| !v.is_empty()),
        "elab should fold DefaultState to a literal, got: {:?}",
        edge.resolved_param_values
    );

    // Parents -----------------------------------------------------------------
    let parents = engine.get_parents("leaf").expect("parents");
    let parent_names: Vec<&str> = parents.iter().map(|m| m.name.as_str()).collect();
    assert!(
        parent_names.contains(&"core"),
        "leaf should have core as a parent, got {parent_names:?}"
    );

    // Vector search by exact name should surface the module itself.
    let hits = engine
        .search_modules("top", 5, None)
        .await
        .expect("search_modules");
    assert!(
        hits.iter().any(|h| h.name == "top"),
        "search hits: {hits:?}"
    );

    // clear_design -----------------------------------------------------------
    engine
        .clear_design(&outcome.manifest.identity.alias)
        .await
        .expect("clear_design");
    assert!(
        engine.get_module("top").unwrap().is_none(),
        "clear_design should remove all modules for the alias"
    );
}

/// Default path: `--top top` but no `--elab`. Slang elaboration is skipped,
/// so `resolved_port_widths` stay empty, but the graph and every other
/// query path must still work end-to-end across the pruned-but-complete
/// set of modules reachable from `top`.
#[tokio::test(flavor = "current_thread")]
async fn build_without_elab_still_indexes_reachable_graph() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = CoreConfig::new(tmp.path());
    cfg.skip_embeddings = true;
    let mut engine = Engine::open(cfg).await.expect("open engine");

    let mut inputs = fixture_inputs(&pickle_root().to_string_lossy());
    inputs.elab = false;
    let outcome = engine
        .build(&inputs)
        .await
        .expect("build without --elab should succeed");
    assert!(outcome.modules_indexed >= 5);

    let ctx = engine
        .get_instance_context("core", "leaf")
        .expect("instance context");
    assert!(!ctx.is_empty(), "core should still instantiate leaf");
    assert!(
        ctx.iter().all(|e| e.resolved_port_widths.is_empty()),
        "no --elab means no resolved port widths, got: {:?}",
        ctx
    );
    assert!(
        ctx.iter().all(|e| e.parent_file_path.ends_with("core.sv")),
        "every instance edge should carry the parent's source file, got: {:?}",
        ctx.iter().map(|e| &e.parent_file_path).collect::<Vec<_>>()
    );

    let path = engine
        .trace_hierarchy_path("top", "leaf")
        .expect("hierarchy path");
    assert!(!path.is_empty());
}

/// Build the dedicated `struct_port.sv` fixture with `--top bus_top
/// --elab` and confirm:
///   1. `resolved_param_values` is empty here (no parameters bound) but
///      the `param_bindings` map likewise stays untouched.
///   2. Scalar ports (`clk_i`) report `total > 0` and an empty `fields`
///      map.
///   3. Packed-struct ports (`req_i`) report `total = 36` with the field
///      breakdown `{addr: 32, prot: 3, valid: 1}`.
///   4. Nested packed-struct ports (`resp_o`) flatten via dot notation:
///      `{status: 8, nested_req.addr: 32, nested_req.prot: 3,
///        nested_req.valid: 1}` and `total = 44`.
///   5. Packed-array-of-structs ports (`req_arr_i`) report
///      `total = 4 * 36 = 144`, `element_count = 4`, an empty top-level
///      `fields` map, and an `element` template carrying the per-element
///      `total = 36` plus the same dot-flattened struct breakdown.
#[tokio::test(flavor = "current_thread")]
async fn elab_top_populates_struct_port_field_breakdown() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = CoreConfig::new(tmp.path());
    cfg.skip_embeddings = true;
    let mut engine = Engine::open(cfg).await.expect("open engine");

    let inputs = ExtractInputs {
        workspace: pickle_root().to_string_lossy().into(),
        targets: vec!["rtl".into()],
        tops: vec!["bus_top".into()],
        elab: true,
        design_alias: Some("struct_port".into()),
        groups: vec![SourceGroupInput {
            files: vec![fixture_file("src/struct_port.sv")],
            include_dirs: vec![],
            defines: vec![],
        }],
        ..Default::default()
    };
    engine.build(&inputs).await.expect("struct_port build");

    let ctx = engine
        .get_instance_context("bus_top", "bus_consumer")
        .expect("instance context");
    assert_eq!(ctx.len(), 1);
    let edge = &ctx[0];

    let clk = edge.resolved_port_widths.get("clk_i").expect("clk_i");
    assert_eq!(clk.total, 1);
    assert!(clk.fields.is_empty());

    let req = edge.resolved_port_widths.get("req_i").expect("req_i");
    assert_eq!(req.total, 36);
    assert_eq!(req.fields.get("addr"), Some(&32));
    assert_eq!(req.fields.get("prot"), Some(&3));
    assert_eq!(req.fields.get("valid"), Some(&1));

    let resp = edge.resolved_port_widths.get("resp_o").expect("resp_o");
    assert_eq!(resp.total, 44);
    assert_eq!(resp.fields.get("status"), Some(&8));
    assert_eq!(resp.fields.get("nested_req.addr"), Some(&32));
    assert_eq!(resp.fields.get("nested_req.prot"), Some(&3));
    assert_eq!(resp.fields.get("nested_req.valid"), Some(&1));

    let arr = edge
        .resolved_port_widths
        .get("req_arr_i")
        .expect("req_arr_i");
    assert_eq!(arr.total, 144);
    assert_eq!(arr.element_count, Some(4));
    assert!(
        arr.fields.is_empty(),
        "top-level fields stay empty for arrays"
    );
    let elem = arr.element.as_deref().expect("element template populated");
    assert_eq!(elem.total, 36);
    assert_eq!(elem.fields.get("addr"), Some(&32));
    assert_eq!(elem.fields.get("prot"), Some(&3));
    assert_eq!(elem.fields.get("valid"), Some(&1));
}

/// Asking for a `--top` that does not exist must hard-error rather than
/// silently producing an empty graph (both with and without `--elab`).
#[tokio::test(flavor = "current_thread")]
async fn unknown_top_hard_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let mut cfg = CoreConfig::new(tmp.path());
    cfg.skip_embeddings = true;
    let mut engine = Engine::open(cfg).await.expect("open engine");

    let mut inputs = fixture_inputs(&pickle_root().to_string_lossy());
    inputs.tops = vec!["definitely_not_a_module".into()];
    let err = engine
        .build(&inputs)
        .await
        .expect_err("build should fail when --top doesn't match");
    let msg = format!("{err}");
    assert!(
        msg.contains("--top") && msg.contains("definitely_not_a_module"),
        "error should mention the offending top: {msg}"
    );
}
