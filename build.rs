#[cfg(not(feature = "slang"))]
fn main() {}

#[cfg(feature = "slang")]
fn main() {
    use cmake;
    use std::env;
    use std::path::PathBuf;

    let dst = cmake::build("../slang");

    // Tell cargo to look for shared libraries in the specified directory
    println!(
        "cargo:rustc-link-search=native={}",
        format!("{}/lib", dst.display())
    );

    // Tell cargo to tell rustc to link the system bzip2
    // shared library.
    println!("cargo:rustc-link-lib=static=svlang");

    // The bindgen::Builder is the main entry point
    // to bindgen, and lets you build up options for
    // the resulting bindings.
    let bindings = bindgen::Builder::default()
        .clang_args([
            "-x".to_string(),
            "c++".to_string(),
            "-std=c++23".to_string(),
            "-DSLANG_BOOST_SINGLE_HEADER".to_string(),
            // "-isysroot/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX14.5.sdk".to_string(),
            format!("-I{}/include/", dst.display()),
            format!("-I../slang/external/")
            // "-I/Library/Developer/CommandLineTools/usr/include/c++/v1".to_string(),
            // "-I/Library/Developer/CommandLineTools/usr/lib/clang/10.0.1/include".to_string()
            // "-isysroot$(xcrun --sdk macosx --show-sdk-path)".to_string()
        ])
        // .allowlist_var(r#"(\w*slang\w*)"#)
        // .allowlist_type(r#"(\w*slang\w*)"#)
        // .allowlist_function(r#"(\w*slang\w*)"#)
        .allowlist_file(format!(
            "{}/include/slang/.*",
            dst.display()
        ))
        // .allowlist_file(format!(
        //     "{}/include/slang/driver/Driver.h",
        //     dst.display()
        // ))
        // .allowlist_file(format!(
        //     "{}/include/slang/ast/Compilation.h",
        //     dst.display()
        // ))
        // The input header we would like to generate
        // bindings for.
        .headers([
            format!("{}/include/slang/ast/Compilation.h", dst.display()),
            format!("{}/include/slang/driver/Driver.h", dst.display()),
            format!("{}/include/slang/util/VersionInfo.h", dst.display()),
        ])
        .enable_cxx_namespaces()
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        // Finish the builder and generate the bindings.
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
