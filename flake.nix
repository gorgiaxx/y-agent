{
  description = "y-agent - Rust-first modular AI Agent framework";

  inputs = {
    flake-utils.url = "github:numtide/flake-utils";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs, flake-utils }@inputs:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs {
          inherit system;
        };

        y-agent = pkgs.callPackage ./package.nix {};
      in
      {
        packages = {
          inherit y-agent;
          default = y-agent;
        };

        devShells.default = pkgs.mkShell {
          name = "y-agent-dev-shell";
          nativeBuildInputs = with pkgs; [
            rustc
            cargo
            cargo-tauri
            rustfmt
            clippy
            nodejs
            pkg-config
          ];

          buildInputs = with pkgs; [
            glib
            glib-networking
            gtk3
            openssl
            webkitgtk_4_1
            libsoup_3
          ];
          # env.NO_STRIP = "true";
        };
      });
}
