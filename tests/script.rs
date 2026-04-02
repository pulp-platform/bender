// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#[cfg(feature = "slang")]
mod tests {
    use assert_cmd::cargo;

    fn run_script(args: &[&str]) -> String {
        let mut full_args = vec!["-d", "tests/pickle", "script"];
        full_args.extend(args);

        let out = cargo::cargo_bin_cmd!()
            .args(&full_args)
            .output()
            .expect("Failed to execute bender binary");

        assert!(
            out.status.success(),
            "script command failed.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        String::from_utf8(out.stdout).expect("stdout must be utf-8")
    }

    #[test]
    fn script_top_filters_unreachable_files() {
        // Without --top: all files present
        let full = run_script(&["--target", "top", "flist-plus"]);
        assert!(full.contains("unused_top.sv"));
        assert!(full.contains("unused_leaf.sv"));

        // With --top top: unreachable files removed
        let trimmed = run_script(&["--target", "top", "--top", "top", "flist-plus"]);
        assert!(trimmed.contains("top.sv"));
        assert!(trimmed.contains("core.sv"));
        assert!(trimmed.contains("leaf.sv"));
        assert!(!trimmed.contains("unused_top.sv"));
        assert!(!trimmed.contains("unused_leaf.sv"));
    }

    #[test]
    fn script_top_multiple_tops() {
        let trimmed = run_script(&[
            "--target",
            "top",
            "--top",
            "top",
            "--top",
            "unused_top",
            "flist-plus",
        ]);
        assert!(trimmed.contains("top.sv"));
        assert!(trimmed.contains("unused_top.sv"));
    }

    #[test]
    fn script_top_empty_keeps_all_files() {
        // Without --top: all files appear
        let full = run_script(&["--target", "top", "flist-plus"]);
        assert!(full.contains("top.sv"));
        assert!(full.contains("core.sv"));
        assert!(full.contains("leaf.sv"));
        assert!(full.contains("unused_top.sv"));
        assert!(full.contains("unused_leaf.sv"));
    }

    /// Regression test: when two files define the same module name, last-wins semantics apply.
    /// The file parsed last (dup_b.sv) wins; the earlier definition (dup_a.sv) is dropped.
    #[test]
    fn script_top_duplicate_module_name_last_wins() {
        // Without --top: both dup files appear (no filtering applied)
        let full = run_script(&["--target", "dup", "flist-plus"]);
        assert!(full.contains("dup_a.sv"));
        assert!(full.contains("dup_b.sv"));
        assert!(full.contains("dup_top.sv"));

        // With --top dup_top: only dup_b.sv (last-wins) and dup_top.sv appear
        let trimmed = run_script(&["--target", "dup", "--top", "dup_top", "flist-plus"]);
        assert!(trimmed.contains("dup_top.sv"));
        assert!(
            trimmed.contains("dup_b.sv"),
            "dup_b.sv (last-wins) missing:\n{trimmed}"
        );
        assert!(
            !trimmed.contains("dup_a.sv"),
            "dup_a.sv (overwritten) should be absent:\n{trimmed}"
        );
    }
}
