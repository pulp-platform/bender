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
}
