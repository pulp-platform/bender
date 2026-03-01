# bender-slang

`bender-slang` provides the C++ bridge between `bender` and the [Slang](https://github.com/MikePopoloski/slang) parser infrastructure, included as a submodule.

It is used by Bender's optional Slang-backed features, most notably the `pickle` command.

## IIS Environment Setup

In the IIS environment on Linux, a newer GCC toolchain is required to build `bender-slang`. Simply copy the provided Cargo configuration file to use the appropriate toolchain:

```sh
cp .cargo/config.toml.iis .cargo/config.toml
```

Then, you can build or install bender with the usual Cargo command:

```sh
cargo install --path . --features slang
```
