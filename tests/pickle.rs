// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#[cfg(feature = "slang")]
mod tests {
    use assert_cmd::cargo;
    fn run_pickle_output(args: &[&str]) -> std::process::Output {
        let mut full_args = vec!["-d", "tests/pickle", "pickle"];
        full_args.extend(args);

        let out = cargo::cargo_bin_cmd!()
            .args(&full_args)
            .output()
            .expect("Failed to execute bender binary");

        assert!(
            out.status.success(),
            "pickle command failed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        out
    }

    fn run_pickle(args: &[&str]) -> String {
        let out = run_pickle_output(args);
        String::from_utf8(out.stdout).expect("stdout must be utf-8")
    }

    fn run_pickle_fail(args: &[&str]) -> std::process::Output {
        let mut full_args = vec!["-d", "tests/pickle", "pickle"];
        full_args.extend(args);

        let out = cargo::cargo_bin_cmd!()
            .args(&full_args)
            .output()
            .expect("Failed to execute bender binary");

        assert!(
            !out.status.success(),
            "pickle command unexpectedly succeeded.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        out
    }

    #[test]
    fn pickle_top_trim_filters_unreachable_modules() {
        let full = run_pickle(&["--target", "top"]);
        assert!(full.contains("module unused_top;"));
        assert!(full.contains("module unused_leaf;"));

        let trimmed = run_pickle(&["--target", "top", "--top", "top"]);
        assert!(trimmed.contains("module top ("));
        assert!(trimmed.contains("module core;"));
        assert!(trimmed.contains("module leaf;"));
        assert!(!trimmed.contains("module unused_top;"));
        assert!(!trimmed.contains("module unused_leaf;"));
    }

    #[test]
    fn pickle_rename_applies_prefix_and_suffix() {
        let renamed = run_pickle(&[
            "--target",
            "top",
            "--top",
            "top",
            "--prefix",
            "p_",
            "--suffix",
            "_s",
            "--expand-macros",
        ]);

        assert!(renamed.contains("module p_top_s ("));
        assert!(renamed.contains("module p_core_s;"));
        assert!(renamed.contains("module p_leaf_s;"));
        assert!(renamed.contains("p_common_pkg_s::is_error(current_state);"));
        assert!(!renamed.contains("`define PKG_IS_ERROR"));
        assert!(!renamed.contains("`define LOG"));
    }

    #[test]
    fn pickle_exclude_rename_keeps_selected_names() {
        let renamed = run_pickle(&[
            "--target",
            "top",
            "--top",
            "top",
            "--prefix",
            "p_",
            "--suffix",
            "_s",
            "--expand-macros",
            "--exclude-rename",
            "top",
            "--exclude-rename",
            "core",
        ]);

        assert!(renamed.contains("module top ("));
        assert!(renamed.contains("module core;"));
        assert!(renamed.contains("module p_leaf_s;"));
        assert!(!renamed.contains("module p_top_s ("));
        assert!(!renamed.contains("module p_core_s;"));
    }

    #[test]
    fn pickle_rename_keeps_undefined_references() {
        let renamed = run_pickle(&[
            "--target",
            "top",
            "--prefix",
            "p_",
            "--suffix",
            "_s",
            "--expand-macros",
        ]);

        assert!(renamed.contains("module p_top_s ("));
        assert!(renamed.contains("undefined_mod u_ext_mod();"));
        assert!(renamed.contains("virtual undefined_intf ext_if;"));
        assert!(renamed.contains("undefined_pkg::undefined_t ext_state;"));
        assert!(!renamed.contains("p_undefined_mod_s"));
        assert!(!renamed.contains("p_undefined_intf_s"));
        assert!(!renamed.contains("p_undefined_pkg_s"));
    }

    #[test]
    fn pickle_rename_renames_named_end_label() {
        let renamed = run_pickle(&["--prefix", "p_", "--suffix", "_s", "--expand-macros"]);

        // Both the module declaration and the named end label must be renamed.
        assert!(renamed.contains("module p_named_end_s;"));
        assert!(renamed.contains("endmodule : p_named_end_s"));
        // The original name must not appear in any end label.
        assert!(!renamed.contains("endmodule : named_end"));
    }

    #[test]
    fn pickle_rename_requires_expand_macros() {
        let out = run_pickle_fail(&["--target", "top", "--prefix", "p_", "--suffix", "_s"]);
        let stderr = String::from_utf8(out.stderr).expect("stderr must be utf-8");

        assert!(stderr.contains("--expand-macros"));
        assert!(stderr.contains("--prefix"));
    }
}
