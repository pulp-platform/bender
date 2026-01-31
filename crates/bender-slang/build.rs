fn main() {
    // Build Slang with CMake into a static library
    let dst = cmake::Config::new("vendor/slang")
        .define("SLANG_INCLUDE_TESTS", "OFF")
        .define("SLANG_INCLUDE_TOOLS", "OFF")
        .define("SLANG_INCLUDE_PYSLANG", "OFF")
        .define("BUILD_SHARED_LIBS", "OFF")
        // Forces installation into 'lib' instead of 'lib64' on some systems.
        .define("CMAKE_INSTALL_LIBDIR", "lib")
        // TODO(fischeti): Check whether mimalloc can/should be enabled again.
        .define("SLANG_USE_MIMALLOC", "OFF")
        // TODO(fischeti): `fmt` currently causes issues on my machine since there is a system-wide installation.
        .define("CMAKE_DISABLE_FIND_PACKAGE_fmt", "ON")
        // TODO(fischeti): Investigate how boost should be handled properly.
        .cxxflag("-DSLANG_BOOST_SINGLE_HEADER=1")
        .static_crt(true)
        .build();

    // Configure Linker to find Slang static library
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    // Note: Linux is case-sensitive, so we use lowercase here.
    // On macOS, the library is called `svLang`, but the linker is case-insensitive there.
    println!("cargo:rustc-link-lib=static=svlang");

    if std::env::var("PROFILE").unwrap() == "debug" {
        println!("cargo:rustc-link-lib=static=fmtd");
    } else {
        println!("cargo:rustc-link-lib=static=fmt");
    }

    // Compile the C++ Bridge
    let mut bridge_build = cxx_build::bridge("src/lib.rs");
    bridge_build
        .file("cpp/slang_bridge.cpp")
        .flag_if_supported("-std=c++20")
        .flag_if_supported("/std:c++20")
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

    // TODO(fischeti): Check whether debug definitions are necessary.
    if std::env::var("PROFILE").unwrap() == "debug" {
        bridge_build.define("SLANG_DEBUG", "1");
    }

    // Linux: we try static linking of libstdc++ to avoid issues on older distros.
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap() == "linux" {
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
    // Windows / MSVC: we force static linking of the CRT to avoid missing DLL issues
    } else if std::env::var("CARGO_CFG_TARGET_ENV").unwrap() == "msvc" {
        bridge_build.static_crt(true);
    }
    // macOS: we leave the default dynamic linking of libc++ as is.

    bridge_build.compile("slang-bridge");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cpp/slang_bridge.cpp");
    println!("cargo:rerun-if-changed=cpp/slang_bridge.h");
}
