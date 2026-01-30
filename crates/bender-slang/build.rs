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
        .build();

    // Configure Linker to find Slang static library
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
    // Note: Linux is case-sensitive, so we use lowercase here.
    // On macOS, the library is called `svLang`, but the linker is case-insensitive there.
    println!("cargo:rustc-link-lib=static=svlang");
    println!("cargo:rustc-link-lib=static=fmtd");

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
        // 1. Slang source headers
        .include("vendor/slang/include")
        // 2. Slang external headers (where boost_unordered.hpp lives)
        .include("vendor/slang/external")
        // 3. CMake build output (where slang_export.h and fmt headers live)
        .include(dst.join("include"));

    // TODO(fischeti): Check whether debug definitions are necessary.
    if std::env::var("PROFILE").unwrap() == "debug" {
        bridge_build.define("SLANG_DEBUG", "1");
    }

    bridge_build.compile("slang-bridge");

    println!("cargo:rerun-if-changed=src/lib.rs");
    println!("cargo:rerun-if-changed=cpp/slang_bridge.cpp");
    println!("cargo:rerun-if-changed=cpp/slang_bridge.h");
}
