{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    flake-parts.url = "github:hercules-ci/flake-parts";
    flake-parts.inputs.nixpkgs-lib.follows = "nixpkgs";
    dnvr.url = "github:dialohq/dnvr/feat/rex-runner";
    dnvr.inputs.nixpkgs.follows = "nixpkgs";
    clickhouse-continuous-backup = {
      url = "github:dialohq/clickhouse-continuous-backup/pause-server";
      flake = false;
    };
  };

  outputs =
    inputs@{ flake-parts, ... }:
    flake-parts.lib.mkFlake { inherit inputs; } {
      systems = [ "aarch64-darwin" ];

      imports = [
        inputs.dnvr.flakeModule
        ./nix/dev-shell.nix
      ];

      perSystem = { pkgs, ... }: {
        formatter = pkgs.nixfmt;
      };
    };
}
