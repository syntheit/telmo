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
  builtIns = [ "net" "bt" "sound" "display" "power" "scale" "system" ];
  customNames = lib.attrNames cfg.popups;

  # Hyprland size per custom popup: "large" is 80% of the monitor.
  hyprSize = popup: if popup.size == "large" then "(monitor_w*0.8) (monitor_h*0.8)" else cfg.hyprland.size;
  popupRules = name: size: map (rule: "${rule}, match:class ^(telmo\\.${name})$") [
    "float 1"
    "center 1"
    "size ${size}"
    "rounding 24"
    "dim_around 1"
    "stay_focused 1"
  ];

  # Only the options that are set, so system.json exists only when needed.
  systemConfig = lib.filterAttrs (_: v: v != null) { inherit (cfg.system) effect logo effects; };

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

    system = {
      effect = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "aurora";
        description = ''
          Starting background effect of the system popup. Only a starting
          value: the choice the popup saves itself (state file) wins.
        '';
      };
      logo = mkOption {
        type = types.nullOr (types.enum [ "apple" "nix" ]);
        default = null;
        description = ''
          Starting logo of the system popup (default: the OS's own). The
          popup's own saved choice (state file) wins.
        '';
      };
      effects = mkOption {
        type = types.nullOr (types.listOf types.str);
        default = null;
        example = [ "aurora" "rain" "snow" ];
        description = "Subset and order of effects the arrow keys cycle through (default: all).";
      };
    };

    popups = mkOption {
      default = { };
      example = { perf = { command = [ "btop" ]; size = "large"; escape = "close"; }; };
      description = ''
        Custom popups: any TUI, opened with `telmo popup <name>`. Written to
        ~/.config/telmo/popups.json. Hotkeys (macOS) and `hyprland.binds` may
        use these names too.
      '';
      type = types.attrsOf (types.submodule {
        options = {
          command = mkOption {
            type = types.listOf types.str;
            description = "Program and arguments, looked up on PATH.";
          };
          size = mkOption {
            type = types.enum [ "normal" "large" ];
            default = "normal";
            description = "normal is 90×22 cells; large is 80% of the screen.";
          };
          escape = mkOption {
            type = types.enum [ "pass" "close" ];
            default = "pass";
            description = ''
              pass: the TUI handles Esc. close: Esc closes the popup (macOS
              host), for TUIs like btop whose own Esc opens a menu.
            '';
          };
        };
      });
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

    (mkIf (cfg.popups != { }) {
      xdg.configFile."telmo/popups.json".text = builtins.toJSON cfg.popups;
    })

    (mkIf (systemConfig != { }) {
      xdg.configFile."telmo/system.json".text = builtins.toJSON systemConfig;
    })

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
        windowrule =
          lib.concatMap (m: popupRules m cfg.hyprland.size) builtIns
          ++ lib.concatMap (m: popupRules m (hyprSize cfg.popups.${m})) customNames;
        bind = lib.mapAttrsToList (m: key: "${key}, exec, telmo popup ${m}") cfg.hyprland.binds;
      };
    })
  ]);
}
