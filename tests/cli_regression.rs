// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;
use std::sync::OnceLock;

use assert_cmd::cargo;
use pretty_assertions::assert_eq;

static SETUP: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();

fn get_test_env() -> &'static (PathBuf, PathBuf) {
    SETUP.get_or_init(|| {
        let root = Path::new("target/tmp_regression");
        let install_root = root.join("golden_install");
        let repo_dir = Path::new("tests/cli_regression").to_path_buf();

        // Install Golden Bender
        let bender_exe = install_root.join("bin").join("bender");

        let golden_branch = std::env::var("BENDER_TEST_GOLDEN_BRANCH")
            .or_else(|_| std::env::var("GITHUB_BASE_REF")) // For GitHub Actions
            .unwrap_or_else(|_| "master".to_string());

        println!("Using golden bender branch: {}", golden_branch);

        if !bender_exe.exists() {
            // Create dir to ensure root exists
            std::fs::create_dir_all(&install_root).expect("Failed to create install dir");

            let status = SysCommand::new("cargo")
                .args(&[
                    "install",
                    "--git",
                    "https://github.com/pulp-platform/bender",
                    "--branch",
                    &golden_branch,
                    "--root",
                    install_root.to_str().unwrap(),
                    "bender",
                ])
                .status()
                .expect("Failed to run cargo install");

            assert!(status.success(), "Failed to install golden bender");
        }

        let bender_exe_abs = std::env::current_dir().unwrap().join(&bender_exe);

        // Checkout the dependencies in the beginning
        let status = SysCommand::new(&bender_exe_abs)
            .arg("checkout")
            .current_dir(&repo_dir)
            .status()
            .expect("Failed to run bender checkout");
        assert!(
            status.success(),
            "Failed to initialize common_cells with bender"
        );

        (bender_exe, repo_dir)
    })
}

fn run_regression(subcommand_args: &[&str]) {
    let (golden_bin, repo_dir) = get_test_env();

    // Construct common args: -d /path/to/repo <SUBCOMMAND_ARGS>
    // We add the directory flag automatically here.
    let mut full_args = vec!["-d", repo_dir.to_str().unwrap()];
    full_args.extend(subcommand_args);

    // Run GOLDEN
    let golden_out = SysCommand::new(golden_bin)
        .args(&full_args)
        .output()
        .expect("Failed to execute golden binary");

    // Run NEW (Current Build)
    let new_out = cargo::cargo_bin_cmd!()
        .args(&full_args)
        .output()
        .expect("Failed to execute new binary");

    // Compare
    let golden_stdout = String::from_utf8_lossy(&golden_out.stdout);
    let new_stdout = String::from_utf8_lossy(&new_out.stdout);

    assert_eq!(golden_stdout, new_stdout);
}

// The Macro generates the tests
macro_rules! regression_tests {
    ($($name:ident: $args:expr),* $(,)?) => {
        $(
            #[test]
            #[ignore]
            fn $name() {
                run_regression($args);
            }
        )*
    };
}

regression_tests! {
    cli_flist:                      &["script", "flist"],
    cli_flist_relative:             &["script", "flist", "--relative-path"],
    cli_flist_plus:                 &["script", "flist-plus"],
    cli_flist_plus_relative:        &["script", "flist-plus", "--define", "CUSTOM_DEFINE=2", "-DSECOND_DEF"],
    cli_flist_plus_only_defines:    &["script", "flist-plus", "--only-defines"],
    cli_flist_plus_only_includes:   &["script", "flist-plus", "--only-includes"],
    cli_flist_plus_only_sources:    &["script", "flist-plus", "--only-sources"],
    cli_flist_plus_nodeps:          &["script", "flist-plus", "--no-deps"],
    cli_flist_plus_cc:              &["script", "flist-plus", "--package", "common_cells"],
    cli_flist_plus_exclude_cc:      &["script", "flist-plus", "--exclude", "common_cells"],
    cli_flist_assume_rtl:           &["script", "flist-plus", "--assume-rtl"],
    cli_vsim:                       &["script", "vsim"],
    cli_vsim_separate:              &["script", "vsim", "--compilation-mode", "separate"],
    cli_vsim_common:                &["script", "vsim", "--compilation-mode", "common"],
    cli_vsim_vlog_arg:              &["script", "vsim", "--vlog-arg", "arg"],
    cli_vsim_vcom_arg:              &["script", "vsim", "--vcom-arg", "arg"],
    cli_vsim_noabort:               &["script", "vsim", "--no-abort-on-error"],
    cli_vcs:                        &["script", "vcs"],
    cli_vcs_vlog_arg:               &["script", "vcs", "--vlog-arg", "arg"],
    cli_vcs_vcom_arg:               &["script", "vcs", "--vcom-arg", "arg"],
    cli_vcs_vhdlan_bin:             &["script", "vcs", "--vhdlan-bin", "arg"],
    cli_vcs_vlogan_bin:             &["script", "vcs", "--vlogan-bin", "arg"],
    cli_verilator:                  &["script", "verilator"],
    cli_synopsys:                   &["script", "synopsys"],
    cli_synopsys_vlog_arg:          &["script", "synopsys", "--vlog-arg", "arg"],
    cli_synopsys_vcom_arg:          &["script", "synopsys", "--vcom-arg", "arg"],
    cli_formality:                  &["script", "formality"],
    cli_riviera:                    &["script", "riviera"],
    cli_riviera_vlog_arg:           &["script", "riviera", "--vlog-arg", "arg"],
    cli_riviera_vcom_arg:           &["script", "riviera", "--vcom-arg", "arg"],
    cli_genus:                      &["script", "genus"],
    cli_vivado:                     &["script", "vivado"],
    cli_vivado_opts:                &["script", "vivado", "--no-simset"],
    cli_vivado_only_defines:        &["script", "vivado", "--only-defines"],
    cli_vivado_only_includes:       &["script", "vivado", "--only-includes"],
    cli_vivado_only_sources:        &["script", "vivado", "--only-sources"],
    cli_vivado_sim:                 &["script", "vivado-sim"],
    cli_vivado_sim_opts:            &["script", "vivado-sim", "--no-simset"],
    cli_vivado_sim_only_defines:    &["script", "vivado-sim", "--only-defines"],
    cli_vivado_sim_only_includes:   &["script", "vivado-sim", "--only-includes"],
    cli_vivado_sim_only_sources:    &["script", "vivado-sim", "--only-sources"],
    cli_precision:                  &["script", "precision"],
    cli_template_json:              &["script", "template_json"],

}
