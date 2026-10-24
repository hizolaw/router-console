self: { config, lib, pkgs, ... }:
let
  cfg = config.services.router-console;
  format = pkgs.formats.toml { };
  address = if lib.hasInfix ":" cfg.listenAddress then "[${cfg.listenAddress}]" else cfg.listenAddress;
  settings = format.generate "router-console.toml" (cfg.settings // {
    listen = "${address}:${toString cfg.port}";
  });
  tokenFile = if cfg.tokenFile == null then "/var/lib/router-console/access.token" else cfg.tokenFile;
  prepareToken = pkgs.writeShellScript "router-console-token" ''
    set -eu
    token=/var/lib/router-console/access.token
    if [ ! -s "$token" ]; then
      umask 077
      ${pkgs.openssl}/bin/openssl rand -hex 32 > "$token.tmp"
      mv "$token.tmp" "$token"
    fi
  '';
in {
  options.services.router-console = {
    enable = lib.mkEnableOption "Router Console";
    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      description = "Router Console package, built with the flake's pinned toolchain.";
    };
    listenAddress = lib.mkOption { type = lib.types.str; default = "127.0.0.1"; };
    port = lib.mkOption { type = lib.types.port; default = 9080; };
    openFirewall = lib.mkOption { type = lib.types.bool; default = false; };
    tokenFile = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      description = "Runtime token file; null generates a persistent root-only token. Never use a Nix store path.";
    };
    environmentFiles = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "Runtime environment files for plugin credentials; prefix with - for an optional file.";
    };
    settings = lib.mkOption {
      type = format.type;
      default = { };
      description = "Console settings. The module owns listen; plugins and title are configurable here. No secrets.";
    };
  };
  config = lib.mkIf cfg.enable {
    assertions = [
      { assertion = !(cfg.settings ? listen); message = "Use services.router-console.listenAddress and port instead of settings.listen."; }
      { assertion = cfg.tokenFile == null || (lib.hasPrefix "/" cfg.tokenFile && !(lib.hasPrefix "/nix/store/" cfg.tokenFile)); message = "router-console.tokenFile must be an absolute runtime path outside the Nix store."; }
    ];
    services.router-console.settings = {
      title = lib.mkDefault "Router Console";
      plugins = lib.mkDefault [ { id = "system"; name = "系统资源"; kind = "system"; interval_secs = 5; } ];
    };
    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];
    systemd.services.router-console-token = lib.mkIf (cfg.tokenFile == null) {
      description = "Initialize Router Console access token if missing";
      before = [ "router-console.service" ];
      serviceConfig = {
        Type = "oneshot";
        ExecStart = prepareToken;
        StateDirectory = "router-console";
        StateDirectoryMode = "0700";
        UMask = "0077";
        RemainAfterExit = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
      };
    };
    systemd.services.router-console = {
      description = "Router Console (read-only)";
      wantedBy = [ "multi-user.target" ];
      after = [ "network.target" ] ++ lib.optional (cfg.tokenFile == null) "router-console-token.service";
      requires = lib.optional (cfg.tokenFile == null) "router-console-token.service";
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} --config ${settings} --token-file %d/access-token";
        LoadCredential = [ "access-token:${tokenFile}" ];
        EnvironmentFile = cfg.environmentFiles;
        DynamicUser = true;
        Restart = "on-failure";
        RestartSec = "3s";
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictAddressFamilies = [ "AF_INET" "AF_INET6" "AF_UNIX" ];
        CapabilityBoundingSet = "";
        UMask = "0077";
        MemoryMax = "64M";
        TasksMax = 32;
      };
    };
  };
}
