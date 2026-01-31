fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap();
    let build_profile = std::env::var("PROFILE").unwrap();

    // Create the configuration builder
    let mut slang_lib = cmake::Config::new("vendor/slang");

    // Apply common settings
    slang_lib
        .define("SLANG_INCLUDE_TESTS", "OFF")
        .define("SLANG_INCLUDE_TOOLS", "OFF")
        .define("SLANG_INCLUDE_PYSLANG", "OFF")
        .define("BUILD_SHARED_LIBS", "OFF")
        // Forces installation into 'lib' instead of 'lib64' on some systems.
        .define("CMAKE_INSTALL_LIBDIR", "lib")
        // TODO(fischeti): `fmt` currently causes issues on my machine since there is a system-wide installation.
        .define("CMAKE_DISABLE_FIND_PACKAGE_fmt", "ON")
        // TODO(fischeti): Investigate how boost should be handled properly.
        .cxxflag("-DSLANG_BOOST_SINGLE_HEADER=1");

    // Windows / MSVC specific flags
    if target_env == "msvc" {
        slang_lib.cxxflag("/EHsc").cxxflag("/utf-8");
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
        // Static Linking Definition
        // Tells Slang headers not to look for DLL import/export symbols.
        .define("SLANG_STATIC_DEFINE", "1")
        // Boost Vendored Mode
        // Tells Slang to use the local 'external/boost_*.hpp' files instead of system Boost.
        // TODO(fischeti): Investigate how boost should be handled properly.
        .define("SLANG_BOOST_SINGLE_HEADER", "1")
        // Include Paths
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
    // Windows / MSVC: we set the appropriate flags for C++20 and exception handling.
    } else if target_env == "msvc" {
        bridge_build
            .flag_if_supported("/std:c++20")
            .flag("/EHsc")
            .flag("/utf-8");
    };
    // macOS: we leave the default dynamic linking of libc++ as is.

    bridge_build.compile("slang-bridge");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cpp/slang_bridge.cpp");
    println!("cargo:rerun-if-changed=cpp/slang_bridge.h");
}
