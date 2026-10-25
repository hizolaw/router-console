self: { config, lib, pkgs, ... }:
let
  cfg = config.services.router-console;
  systemAuth = cfg.authentication == "system";
  socket = "/run/router-console-auth/auth.sock";
  format = pkgs.formats.toml { };
  address = if lib.hasInfix ":" cfg.listenAddress then "[${cfg.listenAddress}]" else cfg.listenAddress;
  settings = format.generate "router-console.toml" (cfg.settings // {
    listen = "${address}:${toString cfg.port}";
  } // lib.optionalAttrs systemAuth { auth = (cfg.settings.auth or { }) // { mode = "system"; inherit socket; }; });
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
    authentication = lib.mkOption { type = lib.types.enum [ "token" "system" ]; default = "token"; description = "Use an access token or explicitly allowed local Unix accounts through PAM."; };
    allowedUsers = lib.mkOption { type = lib.types.listOf lib.types.str; default = [ "root" ]; description = "Local accounts permitted to authenticate when authentication is system."; };
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
      { assertion = !(cfg.settings ? auth && cfg.settings.auth ? mode) || cfg.settings.auth.mode == cfg.authentication; message = "Set router-console.authentication to select the authentication mode."; }
      { assertion = !systemAuth || (cfg.allowedUsers != [] && lib.all (user: builtins.match "[a-zA-Z0-9_-]{1,64}" user != null) cfg.allowedUsers); message = "router-console.allowedUsers must contain explicit local account names."; }
      { assertion = !systemAuth || cfg.tokenFile == null; message = "router-console system authentication cannot use tokenFile."; }
      { assertion = !(cfg.settings ? listen); message = "Use services.router-console.listenAddress and port instead of settings.listen."; }
      { assertion = cfg.tokenFile == null || (lib.hasPrefix "/" cfg.tokenFile && !(lib.hasPrefix "/nix/store/" cfg.tokenFile)); message = "router-console.tokenFile must be an absolute runtime path outside the Nix store."; }
    ];
    users.groups.router-console-auth = lib.mkIf systemAuth { };
    users.groups.router-console = lib.mkIf systemAuth { };
    users.users.router-console = lib.mkIf systemAuth { isSystemUser = true; group = "router-console"; extraGroups = [ "router-console-auth" ]; };
    security.pam.services.router-console = lib.mkIf systemAuth {
      text = ''
        auth required ${pkgs.pam}/lib/security/pam_unix.so
        account required ${pkgs.pam}/lib/security/pam_unix.so
      '';
    };
    systemd.services.router-console-auth = lib.mkIf systemAuth {
      description = "Restricted local PAM authentication for Router Console";
      before = [ "router-console.service" ];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/router-console-auth --socket ${socket} --pam-library ${pkgs.pam}/lib/libpam.so.0 --allowed-users ${lib.escapeShellArg (lib.concatStringsSep "," cfg.allowedUsers)} --pam-service router-console";
        User = "root"; Group = "router-console-auth";
        RuntimeDirectory = "router-console-auth"; RuntimeDirectoryMode = "0750";
        UMask = "0007"; Restart = "on-failure"; RestartSec = "3s";
        NoNewPrivileges = true; ProtectSystem = "strict"; ProtectHome = true;
        PrivateTmp = true; PrivateDevices = true; PrivateNetwork = true;
        RestrictAddressFamilies = [ "AF_UNIX" ]; CapabilityBoundingSet = "";
        ProtectKernelTunables = true; ProtectKernelModules = true; ProtectControlGroups = true;
        MemoryMax = "32M"; TasksMax = 8;
      };
    };
    services.router-console.settings = {
      title = lib.mkDefault "Router Console";
      plugins = lib.mkDefault [ { id = "system"; name = "系统资源"; kind = "system"; interval_secs = 5; } ];
    };
    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ cfg.port ];
    systemd.services.router-console-token = lib.mkIf (!systemAuth && cfg.tokenFile == null) {
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
      after = [ "network.target" ] ++ lib.optional (!systemAuth && cfg.tokenFile == null) "router-console-token.service" ++ lib.optional systemAuth "router-console-auth.service";
      requires = lib.optional (!systemAuth && cfg.tokenFile == null) "router-console-token.service";
      wants = lib.optional systemAuth "router-console-auth.service";
      serviceConfig = {
        ExecStart = "${lib.getExe cfg.package} --config ${settings}" + lib.optionalString (!systemAuth) " --token-file %d/access-token";
        LoadCredential = lib.optionals (!systemAuth) [ "access-token:${tokenFile}" ];
        EnvironmentFile = cfg.environmentFiles;
        DynamicUser = !systemAuth;
        User = lib.mkIf systemAuth "router-console";
        Group = lib.mkIf systemAuth "router-console";
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
