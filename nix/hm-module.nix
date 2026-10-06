self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.telmo;
  inherit (lib) mkEnableOption mkIf mkMerge mkOption types;
  isDarwin = pkgs.stdenv.hostPlatform.isDarwin;
  modules = [ "net" "bt" "sound" ];

  # Where the app is run from. With a signing identity it's a signed copy in
  # ~/Applications, so macOS keeps Location/Bluetooth permissions across
  # rebuilds; otherwise straight from the store.
  storeApp = "${cfg.package}/Applications/Telmo.app";
  signedApp = "${config.home.homeDirectory}/Applications/Telmo.app";
  app = if cfg.signingIdentity == null then storeApp else signedApp;
in
{
  options.programs.telmo = {
    enable = mkEnableOption "telmo popups for network, bluetooth and sound";

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.telmo;
      description = "The telmo package (dispatcher, modules and, on macOS, Telmo.app).";
    };

    signingIdentity = mkOption {
      type = types.nullOr types.str;
      default = null;
      example = "Developer ID Application: Jane Doe (ABCDE12345)";
      description = ''
        macOS: codesign identity for a copy of Telmo.app in ~/Applications.
        A stable signature keeps the Location and Bluetooth permissions
        across rebuilds.
      '';
    };

    hyprland = {
      enable = mkOption {
        type = types.bool;
        default = config.wayland.windowManager.hyprland.enable or false;
        description = "Add Hyprland window rules (and binds) for the popups.";
      };
      binds = mkOption {
        type = types.attrsOf types.str;
        default = { };
        example = { net = "SUPER, N"; bt = "SUPER, B"; sound = "SUPER, M"; };
        description = "Hyprland key per module, e.g. `net = \"SUPER, N\"`.";
      };
      size = mkOption {
        type = types.str;
        default = "1000 580";
        description = "Popup window size in pixels (about 90×22 cells).";
      };
    };
  };

  config = mkIf cfg.enable (mkMerge [
    { home.packages = [ cfg.package ]; }

    (mkIf isDarwin {
      launchd.agents.telmo = {
        enable = true;
        config = {
          ProgramArguments = [ "${app}/Contents/MacOS/Telmo" ];
          RunAtLoad = true;
          KeepAlive = true;
          ProcessType = "Interactive";
        };
      };
    })

    (mkIf (isDarwin && cfg.signingIdentity != null) {
      home.activation.telmoSignApp = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
        # The marker lives outside the bundle: files added after signing break the seal.
        marker="${config.xdg.stateHome}/telmo/app-source"
        if [ "$(cat "$marker" 2>/dev/null)" != "${storeApp}" ]; then
          run mkdir -p "${config.home.homeDirectory}/Applications"
          run rm -rf "${signedApp}"
          # -L: the package is a symlinkJoin; codesign needs real files.
          run cp -RL "${storeApp}" "${signedApp}"
          run chmod -R u+w "${signedApp}"
          # No hardened runtime: it would need extra entitlements for Location.
          run /usr/bin/codesign --force --deep \
            --sign ${lib.escapeShellArg cfg.signingIdentity} "${signedApp}"
          run mkdir -p "$(dirname "$marker")"
          echo "${storeApp}" > "$marker"
          run /bin/launchctl kickstart -k "gui/$(id -u)/org.nix-community.home.telmo" || true
        fi
      '';
    })

    (mkIf (!isDarwin && cfg.hyprland.enable) {
      wayland.windowManager.hyprland.settings = {
        windowrule = lib.concatMap (m: [
          "float 1, match:class ^(telmo\\.${m})$"
          "center 1, match:class ^(telmo\\.${m})$"
          "size ${cfg.hyprland.size}, match:class ^(telmo\\.${m})$"
          "rounding 24, match:class ^(telmo\\.${m})$"
          "dim_around 1, match:class ^(telmo\\.${m})$"
          "stay_focused 1, match:class ^(telmo\\.${m})$"
        ]) modules;
        bind = lib.mapAttrsToList (m: key: "${key}, exec, telmo popup ${m}") cfg.hyprland.binds;
      };
    })
  ]);
}
