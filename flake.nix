{
  description = "captui - terminal screen/window/region recorder with sound (wlroots)";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "captui";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          meta = {
            description = "Terminal screen/window/region recorder with sound (wlroots)";
            license = pkgs.lib.licenses.gpl3Plus;
            mainProgram = "captui";
          };
        };

        devShells.default = pkgs.mkShell {
          # Runtime tools captui shells out to. Once the recorder spawns them,
          # the package should wrapProgram these onto PATH too.
          packages = with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
            cargo-audit
            cargo-deny
            wf-recorder
            wl-screenrec
            slurp
            wlr-randr
            pipewire
            wireplumber
          ];
        };
      });
}
