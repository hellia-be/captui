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
        # Tools captui shells out to at runtime. wrapProgram puts these on the
        # packaged binary's PATH; the devShell reuses the same list.
        runtimeDeps = with pkgs; [
          wf-recorder
          wl-screenrec
          slurp
          wlr-randr
          pipewire # pw-record, pw-dump
          wireplumber # wpctl
          pulseaudio # pactl, for the null-sink mix when both output and input are chosen
          ffmpeg # concat segments into one file when a recording was paused
        ];
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "captui";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          buildFeatures = [ "identify" ];
          nativeBuildInputs = [ pkgs.makeWrapper ];
          postInstall = ''
            wrapProgram $out/bin/captui \
              --prefix PATH : ${pkgs.lib.makeBinPath runtimeDeps}
          '';
          meta = {
            description = "Terminal screen/window/region recorder with sound (wlroots)";
            license = pkgs.lib.licenses.gpl3Plus;
            mainProgram = "captui";
          };
        };

        devShells.default = pkgs.mkShell {
          packages = (with pkgs; [
            cargo
            rustc
            clippy
            rustfmt
            rust-analyzer
            cargo-audit
            cargo-deny
          ]) ++ runtimeDeps;
        };
      });
}
