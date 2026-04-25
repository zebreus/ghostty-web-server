{
  description = "ghostty-web-server – dev shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = nixpkgs.legacyPackages.${system};
      in
      {
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [
            # Rust toolchain (stable channel; targets installed via rustup if needed).
            rustc
            cargo
            rustfmt
            clippy
            # Yew client tooling.
            trunk
            wasm-bindgen-cli
            # Cross-compile + watch helpers.
            cargo-watch
            cargo-zigbuild
            zig
            # Useful while iterating.
            pkg-config
          ];
        };
      });
}
