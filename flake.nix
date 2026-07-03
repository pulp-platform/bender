{
  description = "Dependency management tool for hardware design projects";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    crane.url = "github:ipetkov/crane";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      crane,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = nixpkgs.legacyPackages.${system};

        # Pre-fetched sources for bender-slang's cmake FetchContent dependencies.
        # The Nix sandbox blocks network access, so we fetch these ourselves and
        # inject them via FETCHCONTENT_SOURCE_DIR_* variables in build.rs.
        slangSrc = pkgs.fetchFromGitHub {
          owner = "MikePopoloski";
          repo = "slang";
          tag = "v11.0";
          hash = "sha256-popHzwX0qwv2POAl7/qX3e//OwJRXGtSl9xogpSn2LI=";
        };
        fmtSrc = pkgs.fetchFromGitHub {
          owner = "fmtlib";
          repo = "fmt";
          tag = "12.1.0";
          hash = "sha256-ZmI1Dv0ZabPlxa02OpERI47jp7zFfjpeWCy1WyuPYZ0=";
        };
        mimallocSrc = pkgs.fetchFromGitHub {
          owner = "microsoft";
          repo = "mimalloc";
          tag = "v3.3.2";
          hash = "sha256-GZ37qQVDe9jgMb4Coe5oKvgaLTspZDlSkS5rdy1MfUU=";
        };

        craneLib = crane.mkLib pkgs;

        # Source filter: cargo sources + tera templates + bender-slang C++/CMake files + test fixtures
        slangCrateFilter = path: _type:
          builtins.match ".*/crates/bender-slang/(cpp/.*|CMakeLists\\.txt|build\\.rs)" path != null;
        teraTemplFilter = path: _type: builtins.match ".*src/script_fmt/.*tera" path != null;
        testFixtureFilter = path: _type: builtins.match ".*/tests/.*" path != null;
        benderFilter = path: type:
          (craneLib.filterCargoSources path type)
          || (teraTemplFilter path type)
          || (slangCrateFilter path type)
          || (testFixtureFilter path type);

        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = benderFilter;
          name = "bender-source";
        };

        commonArgs = {
          inherit src;
          strictDeps = true;
          nativeBuildInputs = with pkgs; [
            cmake
            python3
          ];
        };

        cargoArtifacts = craneLib.buildDepsOnly commonArgs;

        bender = craneLib.buildPackage (commonArgs // {
          inherit cargoArtifacts;

          nativeCheckInputs = [ pkgs.gitMinimal ];

          # Point build.rs at pre-fetched FetchContent sources
          SLANG_SRC_DIR = slangSrc;
          FMT_SRC_DIR = fmtSrc;
          MIMALLOC_SRC_DIR = mimallocSrc;

          # owo-colors wraps test assertions in ANSI codes when TERM is set,
          # causing string-matching tests to fail in the sandbox.
          preCheck = "export NO_COLOR=1";

          postCheck = ''
            patchShebangs --build tests
            BENDER="$PWD/target/''${CARGO_BUILD_TARGET:-}/release/bender" tests/run_all.sh
          '';
        });
      in
      {
        packages = {
          default = bender;
          bender = bender;
        };
      }
    );
}
