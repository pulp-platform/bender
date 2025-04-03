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
        teraTemplFilter = path: _type: builtins.match ".*src/script_fmt/.*tera" path != null;
        benderFilter = path: type: ((craneLib.filterCargoSources path type) || (teraTemplFilter path type));

        craneLib = crane.mkLib pkgs;
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = benderFilter;
          name = "bender-source";
        };

        bender = craneLib.buildPackage {
          inherit src;
          strictDeps = true;
        };
      in
      {
        packages = {
          default = bender;
          bender = bender;
        };
      }
    );
}
