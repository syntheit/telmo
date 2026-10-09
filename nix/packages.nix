{ pkgs, crane }:
let
  inherit (pkgs) lib stdenv;
  craneLib = crane.mkLib pkgs;
  common = {
    # Cargo sources plus data files compiled in with include_str! (the System logos).
    src = lib.cleanSourceWith {
      src = ../.;
      filter = path: type: craneLib.filterCargoSources path type || lib.hasSuffix ".json" path;
    };
    strictDeps = true;
    nativeBuildInputs = [ pkgs.pkg-config ];
    buildInputs = lib.optionals stdenv.hostPlatform.isLinux [ pkgs.libpulseaudio pkgs.dbus ];
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
  telmo-host = pkgs.callPackage ./host.nix { };
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
  telmo-display = module "display" "Display popup: brightness, Night Shift, scaling";
  telmo-power = module "power" "Power popup: battery, Low Power Mode, keep awake";
  telmo-helper = craneLib.buildPackage (
    common
    // {
      inherit cargoArtifacts;
      pname = "telmo-helper";
      cargoExtraArgs = "-p telmo-helper";
      doCheck = false;
      meta = {
        description = "Privileged helper for Wi-Fi auto-join (macOS, optional)";
        mainProgram = "telmo-helper";
      };
    }
  );
  telmo-scale = module "scale" "Weigh things on a Force Touch trackpad";
  telmo-system = module "system" "System popup: lock, sleep, restart, effects";
  telmo-clipboard = module "clipboard" "Clipboard history: text, links, colors, images, files";
  telmo = pkgs.symlinkJoin {
    name = "telmo";
    paths = [ telmo-cli telmo-net telmo-bt telmo-sound telmo-display telmo-power telmo-scale telmo-system telmo-clipboard ] ++ lib.optional stdenv.hostPlatform.isDarwin telmo-host;
    meta.mainProgram = "telmo";
  };
  default = telmo;
}
