# Installation

Bender is a single standalone binary. You can either use our recommended shell installer, download a precompiled version, or build it from source.

## Recommended: Shell Installer

The fastest way to install Bender is using our shell script. It detects your operating system and architecture, downloads the latest release, and places it in your path (typically `~/.cargo/bin`).

```sh
curl --proto '=https' --tlsv1.2 https://pulp-platform.github.io/bender/init -sSf | sh
```

### Installing a Specific Version
If you need a specific version of Bender (e.g., `0.31.0`), you can pass it as an argument:
```sh
curl --proto '=https' --tlsv1.2 https://pulp-platform.github.io/bender/init -sSf | sh -s -- 0.31.0
```

## Alternative: Build from Source

If you prefer building your own binary, you will need to [install Rust](https://rustup.rs/).

### Using Cargo
You can install the latest official release directly from [crates.io](https://crates.io/crates/bender):

```sh
cargo install bender
```

> **Note:** By default, Bender includes the `pickle` command which is backed by [Slang](https://github.com/MikePopoloski/slang). This requires a **C++20 compliant compiler** and increases build time significantly. To build without this feature, run:
> ```sh
> cargo install bender --no-default-features
> ```

### From Local Source
If you have cloned the repository, you can install the local version by running the following command from the project root:

```sh
cargo install --path .
```

## Linux Distributions

Some Linux distributions provide Bender through their package managers:

- **ArchLinux:** [Bender on the AUR](https://aur.archlinux.org/packages/bender)

## Verifying Installation

After installation, verify that Bender is available in your terminal:

```sh
bender --version
```

## Shell Completion

Bender supports shell completion for Bash, Zsh, Fish, and PowerShell. To enable it, use the `completion` command and source the output in your shell's configuration file.

For example, for **Zsh**:
```sh
bender completion zsh > ~/.bender_completion.zsh
echo "source ~/.bender_completion.zsh" >> ~/.zshrc
```
