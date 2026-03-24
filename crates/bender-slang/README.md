# bender-slang

> **Internal crate:** `bender-slang` is an internal crate of [Bender](https://github.com/pulp-platform/bender). It does not provide a stable public API — breaking changes may occur at any time without notice.

`bender-slang` provides the C++ bridge between `bender` and the [Slang](https://github.com/MikePopoloski/slang) SystemVerilog parser. It is used by Bender's `pickle` command.

## Building

Building this crate requires a C++20-capable compiler and CMake. The Slang library and its dependencies (fmt, mimalloc) are fetched and built automatically via CMake's FetchContent — no manual setup is required.

## IIS Environment Setup

In the IIS environment on Linux, a newer GCC toolchain is required to build `bender-slang`. Simply copy the provided Cargo configuration file to use the appropriate toolchain:

```sh
cp .cargo/config.toml.iis .cargo/config.toml
```

Then, build or install bender with the usual Cargo command:

```sh
cargo install --path .
```
