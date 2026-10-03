{
  description = "crt: read code by how it behaves";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

  outputs =
    { self, nixpkgs }:
    let
      systems = [
        "aarch64-darwin"
        "x86_64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
      cargo = builtins.fromTOML (builtins.readFile ./crates/crt-cli/Cargo.toml);
    in
    {
      packages = forAllSystems (pkgs: {
        default = self.packages.${pkgs.stdenv.hostPlatform.system}.crt;
        crt = pkgs.rustPlatform.buildRustPackage {
          pname = "crt";
          inherit (cargo.package) version;
          src = pkgs.lib.fileset.toSource {
            root = ./.;
            fileset = pkgs.lib.fileset.unions [
              ./Cargo.toml
              ./Cargo.lock
              ./crates
            ];
          };
          cargoLock.lockFile = ./Cargo.lock;
          cargoBuildFlags = [
            "-p"
            "crt-cli"
          ];
          # The test suite drives Neovim, VS Code and local HTTP servers; CI
          # runs it. The package only builds the binary.
          doCheck = false;
          meta = {
            description = "Explains code per line by how it behaves; a language server for Neovim and VS Code";
            homepage = "https://github.com/esh2n/code-reading-tool";
            license = pkgs.lib.licenses.mit;
            mainProgram = "crt";
          };
        };
      });
    };
}
