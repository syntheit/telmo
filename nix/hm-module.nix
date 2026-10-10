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
  builtIns = [ "net" "bt" "sound" "display" "power" "scale" "system" "clipboard" ];
  customNames = lib.attrNames cfg.popups;

  # Hyprland size per custom popup: "large" is 80% of the monitor.
  hyprSize = popup: if popup.size == "large" then "(monitor_w*0.8) (monitor_h*0.8)" else cfg.hyprland.size;
  # The clipboard popup is 20% bigger than the others: "W H" in pixels.
  clipboardSize =
    let
      parts = lib.splitString " " cfg.hyprland.size;
      bigger = n: toString (builtins.floor (lib.toInt n * 1.2));
    in
    if lib.length parts == 2 then "${bigger (lib.elemAt parts 0)} ${bigger (lib.elemAt parts 1)}" else cfg.hyprland.size;
  sizeFor = name: if name == "clipboard" then clipboardSize else cfg.hyprland.size;
  popupRules = name: size: map (rule: "${rule}, match:class ^(telmo\\.${name})$") [
    "float 1"
    "center 1"
    "size ${size}"
    "rounding 24"
    "dim_around 1"
    "stay_focused 1"
  ];

  # The launcher popup is 66×14 cells: the default size scaled down to that.
  launcherSize =
    let
      parts = lib.splitString " " cfg.hyprland.size;
      scaled = n: by: toString (builtins.floor (lib.toInt n * by / 90.0));
    in
    if lib.length parts == 2 then "${scaled (lib.elemAt parts 0) 66} ${toString (builtins.floor (lib.toInt (lib.elemAt parts 1) * 14 / 22.0))}" else cfg.hyprland.size;
  # Title-cased modifier for the key hints: "SUPER ALT" -> "Super Alt".
  titleCase = s: lib.concatStringsSep " " (map (w: lib.toUpper (lib.substring 0 1 w) + lib.toLower (lib.substring 1 (-1) w)) (lib.splitString " " s));

  # Only the options that are set, so system.json exists only when needed.
  systemConfig = lib.filterAttrs (_: v: v != null) { inherit (cfg.system) effect logo effects rebuild; };

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
      rebuild = mkOption {
        type = types.nullOr (types.listOf types.str);
        default = null;
        example = [ "/run/current-system/sw/bin/darwin-rebuild" "switch" "--flake" "/Users/me/nix#host" "--substituters" "https://cache.nixos.org" ];
        description = ''
          Command the system popup's `u` key runs as a background rebuild,
          without elevation: telmo runs it through `sudo` in the popup's own
          terminal (Touch ID on macOS, the password on Linux), then keeps it
          going in the background. Progress and the result show in the
          popup's footer. Unset: `u` says so.
        '';
      };
    };

    clipboard = {
      enable = mkOption {
        type = types.bool;
        default = false;
        description = ''
          Keep a clipboard history for the clipboard popup. Opt-in because it
          runs a watcher that stores everything you copy, passwords included,
          on this machine only. Linux: a systemd user service running
          `wl-paste --watch`. macOS: Telmo.app watches the pasteboard.
        '';
      };
      maxItems = mkOption {
        type = types.int;
        default = 50;
        description = "Unpinned items kept. Pins don't count.";
      };
      maxDays = mkOption {
        type = types.int;
        default = 30;
        description = "Unpinned items older than this many days are dropped. Pins never expire.";
      };
    };

    launcher = {
      keys = mkOption {
        type = types.attrsOf types.str;
        default = { };
        example = { t = "Ghostty"; w = "Zen Browser"; };
        description = ''
          One letter per app. The letter is a global hotkey for the app (on
          Linux, Hyprland binds below; on macOS wire it to
          `telmo-launcher open "<app>"` in your hotkey daemon) and, typed in
          the launcher, puts that app on top. The list shows it on the right.
        '';
      };
      hotkeyLabel = mkOption {
        type = types.str;
        default = if isDarwin then "fn" else titleCase cfg.launcher.hyprlandModifier;
        defaultText = lib.literalExpression ''"fn" on macOS, else the Hyprland modifier'';
        example = "Super";
        description = "What the launcher shows before an app's letter: `fn T`.";
      };
      search = mkOption {
        type = types.attrsOf types.str;
        default = {
          nix = "https://search.nixos.org/packages?query=%s";
          gh = "https://github.com/search?q=%s";
          yt = "https://www.youtube.com/results?search_query=%s";
          g = "https://www.google.com/search?q=%s";
        };
        description = "Web search engines for `?name words`; `%s` stands for the words.";
      };
      defaultSearch = mkOption {
        type = types.nullOr types.str;
        default = null;
        example = "gh";
        description = "Engine for `?words` without a name (default: nix, else the first by name).";
      };
      hyprlandModifier = mkOption {
        type = types.str;
        default = "SUPER ALT";
        description = "Hyprland modifier for the app hotkeys: `<modifier>, <letter>, exec, telmo-launcher open \"<app>\"`.";
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

    (mkIf cfg.clipboard.enable {
      xdg.configFile."telmo/clipboard.json".text = builtins.toJSON {
        inherit (cfg.clipboard) maxItems maxDays;
      };
    })

    {
      xdg.configFile."telmo/launcher.json".text = builtins.toJSON (
        {
          inherit (cfg.launcher) keys hotkeyLabel search;
        }
        // lib.optionalAttrs (cfg.launcher.defaultSearch != null) { defaultSearch = cfg.launcher.defaultSearch; }
      );
    }

    (mkIf (!isDarwin && cfg.hyprland.enable) {
      wayland.windowManager.hyprland.settings = {
        windowrule = popupRules "launcher" launcherSize;
        bind = lib.mapAttrsToList (
          letter: app: "${cfg.launcher.hyprlandModifier}, ${letter}, exec, ${cfg.package}/bin/telmo-launcher open ${lib.escapeShellArg app}"
        ) cfg.launcher.keys;
      };
    })

    (mkIf (!isDarwin && cfg.clipboard.enable) {
      systemd.user.services.telmo-clipboard = {
        Unit = {
          Description = "telmo clipboard history watcher";
          After = [ "graphical-session.target" ];
          PartOf = [ "graphical-session.target" ];
        };
        Service = {
          ExecStart = "${pkgs.wl-clipboard}/bin/wl-paste --watch ${cfg.package}/bin/telmo-clipboard ingest-wayland";
          Restart = "on-failure";
          Environment = [ "PATH=${lib.makeBinPath [ pkgs.wl-clipboard ]}:/run/current-system/sw/bin" ];
        };
        Install.WantedBy = [ "graphical-session.target" ];
      };
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
          lib.concatMap (m: popupRules m (sizeFor m)) builtIns
          ++ lib.concatMap (m: popupRules m (hyprSize cfg.popups.${m})) customNames;
        bind = lib.mapAttrsToList (m: key: "${key}, exec, telmo popup ${m}") cfg.hyprland.binds;
      };
    })
  ]);
}
