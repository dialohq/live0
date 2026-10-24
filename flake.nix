{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
    dnvr.url = "github:dialohq/dnvr";
    dnvr.inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "aarch64-darwin" ];

      imports = [ inputs.dnvr.flakeModule ];

      perSystem = { config, pkgs, ... }: {
        formatter = pkgs.nixfmt;

        dnvr.shells.default = {
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rust-analyzer
            rustfmt
            nixfmt
          ];
        };
      };
    };
}
