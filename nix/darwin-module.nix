self:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.telmo-helper;
  inherit (lib) mkEnableOption mkIf mkOption types;
in
{
  options.services.telmo-helper = {
    enable = mkEnableOption "the telmo privileged helper (root daemon for Wi-Fi auto-join)";

    user = mkOption {
      type = types.str;
      description = "The only user whose Telmo app may talk to the helper.";
    };

    teamId = mkOption {
      type = types.strMatching "[A-Z0-9]{10}";
      example = "ABCDE12345";
      description = "Apple Developer team ID the Telmo app is signed with. Peers signed by anyone else are refused.";
    };

    package = mkOption {
      type = types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.telmo-helper;
      description = "The telmo-helper package.";
    };
  };

  config = mkIf cfg.enable {
    assertions = [
      {
        assertion = config.users.users ? ${cfg.user} && config.users.users.${cfg.user}.uid != null;
        message = "services.telmo-helper.user must be a nix-darwin user with an explicit uid (users.users.${cfg.user}.uid).";
      }
    ];

    launchd.daemons.telmo-helper.serviceConfig = {
      Label = "io.github.syntheit.telmo-helper";
      ProgramArguments = [
        "${cfg.package}/bin/telmo-helper"
        "--uid"
        (toString config.users.users.${cfg.user}.uid)
        "--team-id"
        cfg.teamId
      ];
      KeepAlive = true;
      RunAtLoad = true;
      ProcessType = "Background";
    };
  };
}
