{
  description = "Acanthus: a Rust toolkit for Wayland shells, panels, lock screens and desktop widgets";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    { self, nixpkgs, crane }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});

      build =
        pkgs:
        let
          inherit (pkgs) lib;
          craneLib = crane.mkLib pkgs;
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./.cargo
              ./rustfmt.toml
              ./crates
              ./examples
              ./xtask
              ./assets
            ];
          };
          runtimeLibs = with pkgs; [
            wayland
            libxkbcommon
            vulkan-loader
            pipewire
            pam
            dbus
          ];
          common = {
            inherit src;
            strictDeps = true;
            pname = "acanthus";
            version = lib.removeSuffix "\n" (builtins.readFile ./version.txt);
            nativeBuildInputs = with pkgs; [
              pkg-config
              clang
              mold
              makeWrapper
            ];
            buildInputs = runtimeLibs ++ (with pkgs; [
              fontconfig
              freetype
            ]);
            LIBCLANG_PATH = "${pkgs.llvmPackages.libclang.lib}/lib";
          };
          cargoArtifacts = craneLib.buildDepsOnly common;
          examples = craneLib.buildPackage (
            common
            // {
              inherit cargoArtifacts;
              pname = "acanthus-examples";
              cargoExtraArgs = "-p hello-bar -p hello-popup -p hello-widget";
              doCheck = false;
              postFixup = ''
                for program in $out/bin/*; do
                  wrapProgram "$program" --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}
                done
              '';
              meta = {
                description = "Example applications of the Acanthus toolkit";
                license = lib.licenses.agpl3Plus;
                platforms = lib.platforms.linux;
              };
            }
          );
        in
        {
          inherit examples;
          clippy = craneLib.cargoClippy (
            common
            // {
              inherit cargoArtifacts;
              cargoClippyExtraArgs = "--workspace --all-targets -- -D warnings";
            }
          );
          fmt = craneLib.cargoFmt { inherit src; };
        };
    in
    {
      packages = forAllSystems (
        pkgs:
        let
          built = build pkgs;
        in
        {
          examples = built.examples;
          default = built.examples;
        }
      );

      checks = forAllSystems (pkgs: build pkgs);

      devShells = forAllSystems (pkgs: {
        default = import ./shell.nix { inherit pkgs; };
      });

      overlays.default = final: prev: {
        acanthus-examples = (build final).examples;
      };

      formatter = forAllSystems (pkgs: pkgs.nixfmt-rfc-style);
    };
}
