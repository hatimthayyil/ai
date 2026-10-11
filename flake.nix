{
  description = "ai: all-in-one AI tool";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs =
    { self, nixpkgs }:
    let
      inherit (nixpkgs) lib;
      forAllSystems = lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
        "x86_64-darwin"
        "aarch64-darwin"
      ];
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          ai = pkgs.rustPlatform.buildRustPackage {
            pname = "ai";
            version = (lib.importTOML ./Cargo.toml).package.version;
            src = lib.fileset.toSource {
              root = ./.;
              fileset = lib.fileset.unions [
                ./Cargo.toml
                ./Cargo.lock
                ./src
                ./tests
              ];
            };
            cargoLock.lockFile = ./Cargo.lock;
            nativeCheckInputs = [ pkgs.git ];
            meta = {
              mainProgram = "ai";
              license = lib.licenses.cc0;
            };
          };
        in
        {
          inherit ai;
          default = ai;
        }
      );
    };
}
