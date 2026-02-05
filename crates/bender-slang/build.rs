// Copyright (c) 2025 ETH Zurich
// Tim Fischer <fischeti@iis.ee.ethz.ch>

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap();
    let build_profile = std::env::var("PROFILE").unwrap();

    // Create the configuration builder
    let mut slang_lib = cmake::Config::new("vendor/slang");

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
    if build_profile == "debug" {
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
        .define("CMAKE_DISABLE_FIND_PACKAGE_Boost", "ON");

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

    // Configure Linker to find Slang static library
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    println!("cargo:rustc-link-lib=static=svlang");

    // Link the additional libraries based on build profile and OS
    match (build_profile.as_str(), target_env.as_str()) {
        ("release", _) | (_, "msvc") => {
            println!("cargo:rustc-link-lib=static=fmt");
            println!("cargo:rustc-link-lib=static=mimalloc");
        }
        ("debug", _) => {
            println!("cargo:rustc-link-lib=static=fmtd");
            println!("cargo:rustc-link-lib=static=mimalloc-debug");
        }
        _ => unreachable!(),
    }

    // Compile the C++ Bridge
    let mut bridge_build = cxx_build::bridge("src/lib.rs");
    bridge_build
        .file("cpp/slang_bridge.cpp")
        .flag_if_supported("-std=c++20")
        .include("vendor/slang/include")
        .include("vendor/slang/external")
        .include(dst.join("include"));

    // Linux: we try static linking of libstdc++ to avoid issues on older distros.
    if target_os == "linux" {
        // Determine the C++ compiler to use. Respect the CXX environment variable if set.
        let compiler = std::env::var("CXX").unwrap_or_else(|_| "g++".to_string());
        // We search for the static libstdc++ file using g++
        let output = std::process::Command::new(&compiler)
            .args(&["-print-file-name=libstdc++.a"])
            .output()
            .expect("Failed to run g++");

        if output.status.success() {
            let path_str = std::str::from_utf8(&output.stdout).unwrap().trim();
            let path = std::path::Path::new(path_str);

            if path.is_absolute() && path.exists() {
                if let Some(parent) = path.parent() {
                    // Add the directory containing libstdc++.a to the link search path
                    println!("cargo:rustc-link-search=native={}", parent.display());
                }

                bridge_build.cpp_set_stdlib(None);
                println!("cargo:rustc-link-lib=static=stdc++");
            } else {
                println!(
                    "cargo:warning=Could not find static libstdc++.a, falling back to dynamic linking"
                );
            }
        }
    }

    // Apply common defines and flags to the bridge build as well
    for (def, value) in common_cxx_defines.iter() {
        bridge_build.define(def, *value);
    }
    for flag in common_cxx_flags.iter() {
        bridge_build.flag(flag);
    }

    bridge_build.compile("slang-bridge");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cpp/slang_bridge.cpp");
    println!("cargo:rerun-if-changed=cpp/slang_bridge.h");
}
