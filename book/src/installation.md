# Installation

To use Bender for a single project, the simplest is to download and use a precompiled binary.  We provide binaries for all current versions of Ubuntu and CentOS, as well as generic Linux, on each release.  Open a terminal and enter the following command:
```sh
curl --proto '=https' --tlsv1.2 https://pulp-platform.github.io/bender/init -sSf | sh
```
The command downloads and executes a script that detects your distribution and downloads the appropriate `bender` binary of the latest release to your current directory.  If you need a specific version of Bender (e.g., `0.21.0`), append ` -s -- 0.21.0` to that command.  Alternatively, you can manually download a precompiled binary from [our Releases on GitHub][releases].

If you prefer building your own binary, you need to [install Rust][rust-installation].  You can then build and install Bender for the current user with the following command:
```sh
cargo install bender
```
If you need a specific version of Bender (e.g., `0.21.0`), append ` --version 0.21.0` to that command.

To install Bender system-wide, you can simply copy the binary you have obtained from one of the above methods to one of the system directories on your `PATH`.  Even better, some Linux distributions have Bender in their repositories.  We are currently aware of:

### [ArchLinux ![aur-shield](https://img.shields.io/aur/version/bender)][aur-bender]

Please extend this list through a PR if you know additional distributions.
