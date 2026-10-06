{
  description = "telmo: keyboard-driven popups for network, bluetooth and sound";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    { self, nixpkgs, crane }:
    let
      systems = [ "aarch64-darwin" "x86_64-darwin" "aarch64-linux" "x86_64-linux" ];
      forAll = f: nixpkgs.lib.genAttrs systems (system: f nixpkgs.legacyPackages.${system});
    in
    {
      packages = forAll (pkgs: import ./nix/packages.nix { inherit pkgs crane; });

      overlays.default = final: _: import ./nix/packages.nix { pkgs = final; inherit crane; };

      homeManagerModules.default = import ./nix/hm-module.nix self;

      devShells = forAll (pkgs: {
        default = pkgs.mkShell {
          packages =
            with pkgs;
            [ cargo rustc clippy rustfmt rust-analyzer cargo-insta pkg-config ]
            ++ lib.optionals stdenv.isLinux [ libpulseaudio ];
        };
      });
    };
}
