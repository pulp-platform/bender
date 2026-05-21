# Installation

Bender is a single standalone binary. You can either use our recommended shell installer, download a precompiled version, or build it from source.

## Recommended: Shell Installer

The fastest way to install Bender is using our shell script. It detects your operating system and architecture, downloads the matching release, and places the `bender` binary in the current directory:

```sh
curl --proto '=https' --tlsv1.2 https://pulp-platform.github.io/bender/init -sSf | sh
```

After the script finishes, you'll find a `bender` executable in your current directory. Move it onto your `PATH` (e.g. `mv bender ~/.local/bin/`) or invoke it as `./bender`.

### Installing a Specific Version
Pass the desired version (e.g. `0.31.0`) as a positional argument:
```sh
curl --proto '=https' --tlsv1.2 https://pulp-platform.github.io/bender/init -sSf | sh -s -- 0.31.0
```

### Global Install
Pass `global` to install into `${CARGO_HOME:-$HOME/.cargo}/bin` instead of the current directory. For v0.32.0 and newer, this also adds the install directory to your `PATH` automatically via the underlying [cargo-dist](https://opensource.axo.dev/cargo-dist/) installer; for older versions, the binary is moved into place but you may need to add the directory to `PATH` manually.

```sh
# Latest release, global install
curl --proto '=https' --tlsv1.2 https://pulp-platform.github.io/bender/init -sSf | sh -s -- global

# Specific version, global install (order of arguments is interchangeable)
curl --proto '=https' --tlsv1.2 https://pulp-platform.github.io/bender/init -sSf | sh -s -- global 0.32.0
```

> **Note:** The installer always overwrites an existing `bender` at the target location without prompting.

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

## Package Managers

Bender is also available through several third-party package managers:

### Homebrew (macOS / Linux)
```sh
brew install bender
```
See the [Homebrew formula](https://formulae.brew.sh/formula/bender) for more details.

### Nix
Bender is packaged in [nixpkgs](https://github.com/NixOS/nixpkgs/blob/master/pkgs/by-name/be/bender/package.nix):
```sh
nix-env -iA nixpkgs.bender
# or, on a flake-enabled system:
nix profile install nixpkgs#bender
```
The repository also ships its own [`flake.nix`](https://github.com/pulp-platform/bender/blob/master/flake.nix), so you can run Bender directly from the latest source without installing:
```sh
nix run github:pulp-platform/bender
```

### Arch Linux (AUR)
```sh
yay -S bender   # or any other AUR helper
```
See [Bender on the AUR](https://aur.archlinux.org/packages/bender) for the package page.

> **Note:** Third-party packages may lag behind the latest [GitHub release](https://github.com/pulp-platform/bender/releases). For the most recent version, prefer the shell installer or `cargo install`.

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
