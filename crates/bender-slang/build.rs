// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

// Generates cpp/compile_flags.txt so that clangd gets the correct include paths
// for the C++ bridge files. The file is written to the cpp/ directory and should
// be gitignored. It is picked up automatically by clangd for all files in that directory.
fn generate_compile_flags(
    manifest_dir: &std::path::Path,
    dst: &std::path::Path,
    includes: &[&std::path::Path],
    defines: &[(&str, &str)],
) {
    use std::ffi::OsStr;

    let Some(target_root) = dst
        .ancestors()
        .find(|p| p.file_name() == Some(OsStr::new("target")))
    else {
        return;
    };

    let flags: Vec<String> = ["-x", "c++", "-std=c++20", "-fno-cxx-modules"]
        .map(str::to_string)
        .into_iter()
        .chain(includes.iter().map(|p| format!("-I{}", p.display())))
        .chain([format!("-I{}", target_root.join("cxxbridge").display())])
        .chain(defines.iter().map(|(k, v)| format!("-D{}={}", k, v)))
        .collect();

    let _ = std::fs::write(
        manifest_dir.join("cpp/compile_flags.txt"),
        flags.join("\n") + "\n",
    );
}

fn main() {
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    // .cargo_vcs_info.json is placed in the package root by cargo during packaging/publish.
    // Writing outside OUT_DIR is forbidden in that context, so skip the clangd helper.
    let in_publish = manifest_dir.join(".cargo_vcs_info.json").exists();

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap();
    let build_profile = std::env::var("PROFILE").unwrap();
    let cmake_profile = match (target_env.as_str(), build_profile.as_str()) {
        // Rust MSVC links against the release CRT;
        // using C++ Debug CRT (/MDd) causes LNK2038 mismatches.
        ("msvc", _) => "RelWithDebInfo",
        (_, "debug") => "Debug",
        _ => "Release",
    };

    // Create the configuration builder
    let mut slang_lib = cmake::Config::new(".");

    // Common defines to give to both Slang and the Bridge
    // Note: It is very important to provide the same defines and flags
    // to both the Slang library build and the C++ bridge build to avoid
    // ABI incompatibilities. Otherwise, this will cause segfaults at runtime.
    let mut common_cxx_defines = vec![
        ("SLANG_USE_MIMALLOC", "1"),
        ("SLANG_USE_THREADS", "1"),
        ("SLANG_BOOST_SINGLE_HEADER", "1"),
    ];

    // Add debug define if in debug build
    if build_profile == "debug" && (target_env != "msvc") {
        common_cxx_defines.push(("SLANG_DEBUG", "1"));
        common_cxx_defines.push(("SLANG_ASSERT_ENABLED", "1"));
    };

    // Common compiler flags
    let common_cxx_flags = if target_env == "msvc" {
        vec!["/std:c++20", "/EHsc", "/utf-8"]
    } else {
        vec!["-std=c++20"]
    };

    // Apply cmake configuration for Slang library
    slang_lib
        .define("SLANG_INCLUDE_TESTS", "OFF")
        .define("SLANG_INCLUDE_TOOLS", "OFF")
        // Forces installation into 'lib' instead of 'lib64' on some systems.
        .define("CMAKE_INSTALL_LIBDIR", "lib")
        // Disable finding system-installed packages, we want to fetch and build them from source.
        .define("CMAKE_DISABLE_FIND_PACKAGE_fmt", "ON")
        .define("CMAKE_DISABLE_FIND_PACKAGE_mimalloc", "ON")
        .define("CMAKE_DISABLE_FIND_PACKAGE_Boost", "ON")
        .profile(cmake_profile);

    // Apply common defines and flags
    for (def, value) in common_cxx_defines.iter() {
        slang_lib.define(def, *value);
        slang_lib.cxxflag(format!("-D{}={}", def, value));
    }
    for flag in common_cxx_flags.iter() {
        slang_lib.cxxflag(flag);
    }

    // Build the slang library
    let dst = slang_lib.build();
    // With FetchContent, cmake builds slang in a _deps subdirectory rather than
    // installing it. Point directly at the FetchContent build/source directories.
    let slang_lib_dir = dst.join("build/_deps/slang-build/lib");
    let slang_include_dir = dst.join("build/_deps/slang-src/include");
    let slang_generated_include_dir = dst.join("build/_deps/slang-build/source");
    let fmt_include_dir = dst.join("build/_deps/fmt-src/include");

    // Generate cpp/compile_flags.txt for clangd IDE support
    if !in_publish {
        generate_compile_flags(
            &manifest_dir,
            &dst,
            &[
                &slang_include_dir,
                &slang_generated_include_dir,
                &dst.join("slang-external"),
                &fmt_include_dir,
            ],
            &common_cxx_defines,
        );
    }

    // Configure Linker to find Slang static library
    println!("cargo:rustc-link-search=native={}", slang_lib_dir.display());
    println!("cargo:rustc-link-lib=static=svlang");

    // Link the additional libraries based on build profile.
    let (fmt_lib, mimalloc_lib) = match (target_env.as_str(), build_profile.as_str()) {
        ("msvc", _) => ("fmt", "mimalloc"),
        (_, "debug") => ("fmtd", "mimalloc-debug"),
        _ => ("fmt", "mimalloc"),
    };

    println!("cargo:rustc-link-lib=static={fmt_lib}");
    println!("cargo:rustc-link-lib=static={mimalloc_lib}");

    if target_os == "windows" {
        println!("cargo:rustc-link-lib=advapi32");
    }

    // Compile the C++ Bridge
    let mut bridge_build = cxx_build::bridge("src/lib.rs");
    bridge_build
        .file("cpp/session.cpp")
        .file("cpp/rewriter.cpp")
        .file("cpp/print.cpp")
        .file("cpp/analysis.cpp")
        .flag_if_supported("-std=c++20")
        .include(&slang_include_dir)
        .include(&slang_generated_include_dir)
        .include(dst.join("slang-external"))
        .include(&fmt_include_dir);

    // Apply common defines and flags to the bridge build as well
    for (def, value) in common_cxx_defines.iter() {
        bridge_build.define(def, *value);
    }
    for flag in common_cxx_flags.iter() {
        bridge_build.flag(flag);
    }

    bridge_build.compile("slang-bridge");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cpp/slang_bridge.h");
    println!("cargo:rerun-if-changed=cpp/session.cpp");
    println!("cargo:rerun-if-changed=cpp/rewriter.cpp");
    println!("cargo:rerun-if-changed=cpp/print.cpp");
    println!("cargo:rerun-if-changed=cpp/analysis.cpp");
}
