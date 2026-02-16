// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#[cfg(feature = "slang")]
mod tests {
    use assert_cmd::cargo;

    fn run_pickle(args: &[&str]) -> String {
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

        String::from_utf8(out.stdout).expect("stdout must be utf-8")
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
            "--target", "top", "--top", "top", "--prefix", "p_", "--suffix", "_s",
        ]);

        assert!(renamed.contains("module p_top_s ("));
        assert!(renamed.contains("module p_core_s;"));
        assert!(renamed.contains("module p_leaf_s;"));
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
}
