// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

#[cfg(feature = "slang")]
mod tests {
    use std::collections::HashSet;

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

    /// Run the script subcommand expecting failure; returns the captured stderr.
    fn run_script_failing(args: &[&str]) -> String {
        let mut full_args = vec!["-d", "tests/pickle", "script"];
        full_args.extend(args);

        let out = cargo::cargo_bin_cmd!()
            .args(&full_args)
            .output()
            .expect("Failed to execute bender binary");

        assert!(
            !out.status.success(),
            "script command unexpectedly succeeded.\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        String::from_utf8(out.stderr).expect("stderr must be utf-8")
    }

    /// Return the path component after the last `/` or `\`. On Windows, bender's source paths
    /// can come out mixed (e.g. `D:\workspace\tests\pickle/src/top.sv`) while incdir paths are
    /// all-backslash because the Bender.yml entry has no embedded separator — so splitting on
    /// `/` alone misses the latter.
    fn basename(path: &str) -> &str {
        match path.rfind(['/', '\\']) {
            Some(i) => &path[i + 1..],
            None => path,
        }
    }

    /// Extract the set of source-file basenames from a flist-plus output.
    /// Filters out `+incdir+` / `+define+` lines so only path lines remain.
    fn source_basenames(output: &str) -> HashSet<&str> {
        output
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("+incdir+") && !l.starts_with("+define+"))
            .map(basename)
            .collect()
    }

    /// Extract the basenames of `+incdir+` directories from a flist-plus output.
    fn incdir_basenames(output: &str) -> HashSet<&str> {
        output
            .lines()
            .map(str::trim)
            .filter_map(|l| l.strip_prefix("+incdir+"))
            .map(basename)
            .collect()
    }

    #[test]
    fn script_top_filters_unreachable_files() {
        // Without --top: all files present
        let full_out = run_script(&["--target", "top", "flist-plus"]);
        let full = source_basenames(&full_out);
        assert!(full.contains("unused_top.sv"));
        assert!(full.contains("unused_leaf.sv"));

        // With --top top: unreachable files removed
        let trimmed_out = run_script(&["--target", "top", "--top", "top", "flist-plus"]);
        let trimmed = source_basenames(&trimmed_out);
        assert!(trimmed.contains("top.sv"));
        assert!(trimmed.contains("core.sv"));
        assert!(trimmed.contains("leaf.sv"));
        assert!(!trimmed.contains("unused_top.sv"));
        assert!(!trimmed.contains("unused_leaf.sv"));
    }

    #[test]
    fn script_top_multiple_tops() {
        let out = run_script(&[
            "--target",
            "top",
            "--top",
            "top",
            "--top",
            "unused_top",
            "flist-plus",
        ]);
        let trimmed = source_basenames(&out);
        assert!(trimmed.contains("top.sv"));
        assert!(trimmed.contains("unused_top.sv"));
    }

    #[test]
    fn script_top_empty_keeps_all_files() {
        // Without --top: all files appear
        let out = run_script(&["--target", "top", "flist-plus"]);
        let full = source_basenames(&out);
        assert!(full.contains("top.sv"));
        assert!(full.contains("core.sv"));
        assert!(full.contains("leaf.sv"));
        assert!(full.contains("unused_top.sv"));
        assert!(full.contains("unused_leaf.sv"));
    }

    /// Default (`--trim-incdirs auto`) trims include dirs iff `--top` is set.
    /// `include/` is used by top.sv (`include "macros.svh"`); `include_unused/` is declared in
    /// the Bender.yml but never resolved through.
    #[test]
    fn script_trim_incdirs_auto() {
        // No --top: both dirs survive.
        let full_out = run_script(&["--target", "top", "flist-plus"]);
        let full_incs = incdir_basenames(&full_out);
        assert!(full_incs.contains("include"));
        assert!(full_incs.contains("include_unused"));

        // With --top: include_unused is dropped, include survives.
        let trimmed_out = run_script(&["--target", "top", "--top", "top", "flist-plus"]);
        let trimmed_incs = incdir_basenames(&trimmed_out);
        assert!(
            trimmed_incs.contains("include"),
            "include/ should survive: {trimmed_incs:?}"
        );
        assert!(
            !trimmed_incs.contains("include_unused"),
            "include_unused/ should be dropped: {trimmed_incs:?}"
        );
    }

    /// `--trim-incdirs always` prunes unused incdirs even without `--top`.
    #[test]
    fn script_trim_incdirs_always_without_top() {
        let out = run_script(&["--target", "top", "--trim-incdirs", "always", "flist-plus"]);
        let incs = incdir_basenames(&out);
        assert!(incs.contains("include"));
        assert!(
            !incs.contains("include_unused"),
            "include_unused/ should be dropped: {incs:?}"
        );

        // File list is untouched — no --top means no reachability filter.
        let files = source_basenames(&out);
        assert!(files.contains("top.sv"));
        assert!(files.contains("unused_top.sv"));
        assert!(files.contains("unused_leaf.sv"));
    }

    /// `--trim-incdirs never` keeps all incdirs even with `--top`.
    #[test]
    fn script_trim_incdirs_never_with_top() {
        let out = run_script(&[
            "--target",
            "top",
            "--top",
            "top",
            "--trim-incdirs",
            "never",
            "flist-plus",
        ]);
        let incs = incdir_basenames(&out);
        assert!(incs.contains("include"));
        assert!(
            incs.contains("include_unused"),
            "include_unused/ should be retained with --trim-incdirs never: {incs:?}"
        );

        // File filtering still happens — unreachable files are still dropped.
        let files = source_basenames(&out);
        assert!(files.contains("top.sv"));
        assert!(!files.contains("unused_top.sv"));
    }

    /// Encrypted RTL (IEEE-1735 protect envelopes) makes slang trip at the surrounding
    /// `endmodule` even though the envelope itself is skipped. The filter must:
    ///  * not abort `bender script --top` because of slang errors in encrypted IP, and
    ///  * preserve the encrypted file in the output even though no internal reference resolves
    ///    to it (its module symbol is hidden behind the protect envelope).
    #[test]
    fn script_top_keeps_encrypted_file() {
        let out = run_script(&[
            "--target",
            "encrypted",
            "--top",
            "encrypted_top",
            "flist-plus",
        ]);
        let files = source_basenames(&out);
        assert!(
            files.contains("encrypted_top.sv"),
            "top file missing: {files:?}"
        );
        assert!(
            files.contains("encrypted_user.sv"),
            "user of encrypted IP missing: {files:?}"
        );
        assert!(
            files.contains("encrypted_ip.sv"),
            "encrypted IP must be force-kept despite parse errors: {files:?}"
        );
    }

    /// `--encrypted drop` removes encrypted files from the output even when `--top` keeps
    /// the rest of the reachable design.
    #[test]
    fn script_encrypted_drop_excludes_encrypted_file() {
        let out = run_script(&[
            "--target",
            "encrypted",
            "--top",
            "encrypted_top",
            "--encrypted",
            "drop",
            "flist-plus",
        ]);
        let files = source_basenames(&out);
        assert!(files.contains("encrypted_top.sv"));
        assert!(files.contains("encrypted_user.sv"));
        assert!(
            !files.contains("encrypted_ip.sv"),
            "encrypted IP should be dropped by --encrypted drop: {files:?}"
        );
    }

    /// `--encrypted error` makes the run abort when any encrypted file is encountered — useful
    /// as a CI lint for codebases that should be encryption-free.
    #[test]
    fn script_encrypted_error_aborts() {
        let stderr = run_script_failing(&[
            "--target",
            "encrypted",
            "--encrypted",
            "error",
            "flist-plus",
        ]);
        assert!(
            stderr.contains("encrypted file(s)") && stderr.contains("--encrypted error"),
            "expected encrypted-lint abort message, got stderr:\n{stderr}"
        );
    }

    /// `--source-annotations` adds a `// UNPARSEABLE: ...` comment above the kept encrypted file
    /// so users reading the script can tell which entries slang couldn't analyze.
    #[test]
    fn script_source_annotations_marks_unparseable() {
        let out = run_script(&[
            "--target",
            "encrypted",
            "--top",
            "encrypted_top",
            "--source-annotations",
            "flist-plus",
        ]);
        assert!(
            out.contains("// UNPARSEABLE: slang reported parse errors"),
            "expected UNPARSEABLE annotation in output:\n{out}"
        );
    }

    /// A file with a real syntax error (no `pragma protect` envelope) should abort the run by
    /// default — `--broken` defaults to `error`, while encrypted IP is auto-tolerated.
    #[test]
    fn script_broken_file_fails_by_default() {
        let stderr = run_script_failing(&["--target", "broken", "--top", "broken", "flist-plus"]);
        assert!(
            stderr.contains("looks like real syntax bugs"),
            "expected real-bug error message, got stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("--broken keep") || stderr.contains("--broken drop"),
            "error should point at the --broken keep/drop escape hatches:\n{stderr}"
        );
    }

    /// `--broken keep` lets users opt into tolerance for non-encrypted parse errors. The broken
    /// file is kept in the output with a warning instead of aborting.
    #[test]
    fn script_broken_keep_includes_broken_file() {
        let out = run_script(&[
            "--target",
            "broken",
            "--top",
            "broken",
            "--broken",
            "keep",
            "flist-plus",
        ]);
        let files = source_basenames(&out);
        assert!(
            files.contains("broken.sv"),
            "broken.sv should survive with --broken keep: {files:?}"
        );
    }

    /// `--broken drop` tolerates broken files but excludes them from the script — useful when
    /// the downstream tool would choke on the broken file but you still want bender to finish.
    #[test]
    fn script_broken_drop_excludes_broken_file() {
        let out = run_script(&[
            "--target",
            "broken",
            "--top",
            "broken",
            "--broken",
            "drop",
            "flist-plus",
        ]);
        let files = source_basenames(&out);
        assert!(
            !files.contains("broken.sv"),
            "broken.sv should be dropped with --broken drop: {files:?}"
        );
    }

    /// Explicit `--broken drop` triggers slang even without `--top`, so users can drop broken
    /// files from a plain `bender script` output without setting up reachability filtering.
    #[test]
    fn script_broken_drop_without_top_excludes_broken_file() {
        let out = run_script(&["--target", "broken", "--broken", "drop", "flist-plus"]);
        let files = source_basenames(&out);
        assert!(
            !files.contains("broken.sv"),
            "broken.sv should be dropped without --top when --broken drop is set: {files:?}"
        );
    }

    /// `--broken keep` is a no-op without `--top` (keeping is what happens anyway when slang
    /// doesn't run). Verifies broken.sv passes through with no warnings emitted.
    #[test]
    fn script_broken_keep_without_top_no_slang() {
        let out = run_script(&["--target", "broken", "--broken", "keep", "flist-plus"]);
        let files = source_basenames(&out);
        assert!(
            files.contains("broken.sv"),
            "broken.sv should pass through with --broken keep: {files:?}"
        );
    }

    /// Regression test: when two files define the same module name, last-wins semantics apply.
    /// The file parsed last (dup_b.sv) wins; the earlier definition (dup_a.sv) is dropped.
    #[test]
    fn script_top_duplicate_module_name_last_wins() {
        // Without --top: both dup files appear (no filtering applied)
        let full_out = run_script(&["--target", "dup", "flist-plus"]);
        let full = source_basenames(&full_out);
        assert!(full.contains("dup_a.sv"));
        assert!(full.contains("dup_b.sv"));
        assert!(full.contains("dup_top.sv"));

        // With --top dup_top: only dup_b.sv (last-wins) and dup_top.sv appear
        let trimmed_out = run_script(&["--target", "dup", "--top", "dup_top", "flist-plus"]);
        let trimmed = source_basenames(&trimmed_out);
        assert!(trimmed.contains("dup_top.sv"));
        assert!(
            trimmed.contains("dup_b.sv"),
            "dup_b.sv (last-wins) missing: {trimmed:?}"
        );
        assert!(
            !trimmed.contains("dup_a.sv"),
            "dup_a.sv (overwritten) should be absent: {trimmed:?}"
        );
    }

    /// Regression test: files whose extension isn't a recognized HDL type (e.g. `.txt`) must
    /// still appear in `flist`/`flist-plus` output. They used to be silently dropped once the
    /// file list switched to iterating per-type source groups.
    #[test]
    fn script_untyped_files_kept_in_flist() {
        for fmt in ["flist", "flist-plus"] {
            let out = run_script(&["--target", "untyped", fmt]);
            let files = source_basenames(&out);
            assert!(
                files.contains("misc_rtl.sv"),
                "typed file missing from {fmt}: {files:?}"
            );
            assert!(
                files.contains("misc_notes.txt"),
                "untyped file must be kept in {fmt}: {files:?}"
            );
        }
    }

    /// Tool scripts that branch on file type must not emit a broken compile command for untyped
    /// files: they comment them out instead. Verify the untyped file appears only on comment
    /// lines and that a skip notice is present.
    #[test]
    fn script_untyped_files_commented_in_tool_scripts() {
        for fmt in [
            "vsim",
            "vcs",
            "synopsys",
            "formality",
            "riviera",
            "genus",
            "precision",
        ] {
            let out = run_script(&["--target", "untyped", fmt]);
            let mentions: Vec<&str> = out
                .lines()
                .filter(|l| l.contains("misc_notes.txt"))
                .collect();
            assert!(
                !mentions.is_empty(),
                "untyped file absent from {fmt} output:\n{out}"
            );
            for line in &mentions {
                assert!(
                    line.trim_start().starts_with('#'),
                    "untyped file must only appear in comments for {fmt}, got line {line:?}\nfull:\n{out}"
                );
            }
            assert!(
                out.contains("Skipping") && out.contains("unknown type"),
                "expected an unknown-type skip notice in {fmt} output:\n{out}"
            );
        }
    }
}
