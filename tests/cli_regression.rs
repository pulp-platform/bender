// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

use assert_cmd::cargo;

use std::path::{Path, PathBuf};
use std::process::Command as SysCommand;
use std::sync::OnceLock;

static SETUP: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();

fn get_test_env() -> &'static (PathBuf, PathBuf) {
    SETUP.get_or_init(|| {
        let root = Path::new("target/tmp_regression");
        let install_root = root.join("golden_install"); // Clean separate dir for the binary
        let repo_dir = Path::new("tests/cli_regression").to_path_buf();

        // 1. Install Golden Bender
        // Cargo install --root X puts the binary at X/bin/bender
        let bender_exe = install_root.join("bin").join("bender");

        if !bender_exe.exists() {
            println!("--- SETUP: Installing Bender (master) as golden reference ---");
            // Create dir to ensure root exists
            std::fs::create_dir_all(&install_root).expect("Failed to create install dir");

            let status = SysCommand::new("cargo")
                .args(&[
                    "install",
                    "--git",
                    "https://github.com/pulp-platform/bender",
                    "--branch",
                    "master",
                    "--root",
                    install_root.to_str().unwrap(),
                    "bender",
                ])
                .status()
                .expect("Failed to run cargo install");

            assert!(status.success(), "Failed to install golden bender");
        }

        let bender_exe_abs = std::env::current_dir().unwrap().join(&bender_exe);

        // Checkout the repo
        println!("--- SETUP: Initializing common_cells ---");
        println! {"Repo dir: {}", repo_dir.display()};
        println! {"Bender exe: {}", bender_exe.display()};
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

    println!("Testing: {} {}", golden_bin.display(), full_args.join(" "));

    // 1. Run GOLDEN
    let golden_out = SysCommand::new(golden_bin)
        .args(&full_args)
        .output()
        .expect("Failed to execute golden binary");

    // 2. Run NEW (Current Build)
    let new_out = cargo::cargo_bin_cmd!()
        .args(&full_args)
        .output()
        .expect("Failed to execute new binary");

    // 3. Compare
    let golden_stdout = String::from_utf8_lossy(&golden_out.stdout);
    let new_stdout = String::from_utf8_lossy(&new_out.stdout);

    assert_eq!(
        golden_stdout,
        new_stdout,
        "STDOUT mismatch.\nCMD: bender {}\n\n--- GOLDEN ---\n{}\n\n--- NEW ---\n{}",
        full_args.join(" "),
        golden_stdout,
        new_stdout
    );
}

// The Macro generates the tests
macro_rules! regression_tests {
    ($($name:ident: $args:expr),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                run_regression($args);
            }
        )*
    };
}

regression_tests! {
    flist:       &["script", "flist"],
    flist_relative: &["script", "flist", "--relative-path"],
    flist_plus_relative: &["script", "flist-plus", "--relative-path"],
    flist_plus_only_defines: &["script", "flist-plus", "--only-defines"],
    flist_plus_only_includes: &["script", "flist-plus", "--only-includes"],
    flist_plus_only_sources: &["script", "flist-plus", "--only-sources"],
    flist_plus:  &["script", "flist-plus"],
    vsim:        &["script", "vsim"],
    vsim_separate: &["script", "vsim", "--compilation-mode", "separate"],
    vsim_common: &["script", "vsim", "--compilation-mode", "common"],
    vsim_vlog_arg: &["script", "vsim", "--vlog-arg", "bubu"],
    vsim_vcom_arg: &["script", "vsim", "--vcom-arg", "bubu"],
    vcs:        &["script", "vcs"],
    vcs_vlog_arg: &["script", "vcs", "--vlog-arg", "bubu"],
    vcs_vcom_arg: &["script", "vcs", "--vcom-arg", "bubu"],
    vcs_vhdlan_bin: &["script", "vcs", "--vhdlan-bin", "bubu"],
    vcs_vlogan_bin: &["script", "vcs", "--vlogan-bin", "bubu"],
    verilator:  &["script", "verilator"],
    synopsys:   &["script", "synopsys"],
    synopsys_vlog_arg: &["script", "synopsys", "--vlog-arg", "bubu"],
    synopsys_vcom_arg: &["script", "synopsys", "--vcom-arg", "bubu"],
    formality:  &["script", "formality"],
    riviera:    &["script", "riviera"],
    riviera_vlog_arg: &["script", "riviera", "--vlog-arg", "bubu"],
    riviera_vcom_arg: &["script", "riviera", "--vcom-arg", "bubu"],
    genus:      &["script", "genus"],
    vivado:      &["script", "vivado"],
    vivado_opts: &["script", "vivado", "--no-simset"],
    vivado_only_defines: &["script", "vivado", "--only-defines"],
    vivado_only_includes: &["script", "vivado", "--only-includes"],
    vivado_only_sources: &["script", "vivado", "--only-sources"],
    vivado_sim:      &["script", "vivado-sim"],
    vivado_sim_opts: &["script", "vivado-sim", "--no-simset"],
    vivado_sim_only_defines: &["script", "vivado-sim", "--only-defines"],
    vivado_sim_only_includes: &["script", "vivado-sim", "--only-includes"],
    vivado_sim_only_sources: &["script", "vivado-sim", "--only-sources"],
    precision:  &["script", "precision"],
    template_json: &["script", "template_json"],

}
