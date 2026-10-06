{ pkgs, crane }:
let
  inherit (pkgs) lib stdenv;
  craneLib = crane.mkLib pkgs;
  common = {
    src = craneLib.cleanCargoSource ../.;
    strictDeps = true;
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = lib.optionals stdenv.isLinux [ pkgs.libpulseaudio ];
  };
  cargoArtifacts = craneLib.buildDepsOnly (common // { pname = "telmo-deps"; });
  module =
    name: description:
    craneLib.buildPackage (
      common
      // {
        inherit cargoArtifacts;
        pname = "telmo-${name}";
        cargoExtraArgs = "-p telmo-${name}";
        doCheck = false;
        meta = { inherit description; mainProgram = "telmo-${name}"; };
      }
    );
in
rec {
  telmo-cli = craneLib.buildPackage (
    common
    // {
      inherit cargoArtifacts;
      pname = "telmo";
      cargoExtraArgs = "-p telmo";
      doCheck = false;
      meta.mainProgram = "telmo";
    }
  );
  telmo-net = module "net" "Network popup: Wi-Fi, Ethernet, VPN, speedtest";
  telmo-bt = module "bt" "Bluetooth popup";
  telmo-sound = module "sound" "Sound popup";
  telmo = pkgs.symlinkJoin {
    name = "telmo";
    paths = [ telmo-cli telmo-net telmo-bt telmo-sound ];
    meta.mainProgram = "telmo";
  };
  default = telmo;
}
