{
  description = "Lightweight Rust router console and NixOS service module";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }: let
    systems = [ "x86_64-linux" "aarch64-linux" ];
    forAllSystems = nixpkgs.lib.genAttrs systems;
  in {
    packages = forAllSystems (system: let
      pkgs = import nixpkgs { inherit system; };
    in {
      default = self.packages.${system}.router-console;
      router-console = pkgs.callPackage ./nix/package.nix { };
    });

    apps = forAllSystems (system: {
      default = {
        type = "app";
        meta.description = "Run Router Console";
        program = "${self.packages.${system}.default}/bin/router-console";
      };
    });

    nixosModules.default = import ./nix/module.nix self;
    nixosModules.router-console = self.nixosModules.default;

    checks = forAllSystems (system: let
      pkgs = import nixpkgs { inherit system; };
      moduleSystem = nixpkgs.lib.nixosSystem {
        inherit system;
        modules = [
          self.nixosModules.default
          ({ ... }: {
            boot.isContainer = true;
            networking.hostName = "console-check";
            system.stateVersion = "24.05";
            services.router-console.enable = true;
            services.router-console.settings.plugins = [ {
              id = "runtime-link";
              name = "Runtime link";
              kind = "link";
              url_env = "ROUTER_CONSOLE_TEST_LINK";
            } ];
          })
        ];
      };
      customTokenSystem = moduleSystem.extendModules {
        modules = [ { services.router-console.tokenFile = "/run/secrets/router-console"; } ];
      };
      testSettings = (pkgs.formats.toml { }).generate "router-console-module-test.toml"
        (moduleSystem.config.services.router-console.settings // { listen = "127.0.0.1:9080"; });
      disabledSystem = moduleSystem.extendModules {
        modules = [ { services.router-console.enable = nixpkgs.lib.mkForce false; } ];
      };
    in {
      package = self.packages.${system}.default;
      module = assert moduleSystem.config.systemd.services.router-console.serviceConfig.DynamicUser;
        assert moduleSystem.config.services.router-console.port == 9080;
        assert !(disabledSystem.config.systemd.services ? router-console);
        assert !(disabledSystem.config.systemd.services ? router-console-token);
        assert !(customTokenSystem.config.systemd.services ? router-console-token);
        assert customTokenSystem.config.systemd.services.router-console.serviceConfig.LoadCredential == [ "access-token:/run/secrets/router-console" ];
        pkgs.runCommand "router-console-module-check" { } ''
          ROUTER_CONSOLE_TOKEN=public-module-test-token-123456 \
            ${self.packages.${system}.default}/bin/router-console --config ${testSettings} --check
          touch $out
        '';
    });

    devShells = forAllSystems (system: let pkgs = import nixpkgs { inherit system; }; in {
      default = pkgs.mkShell {
        packages = with pkgs; [ cargo rustc rustfmt clippy gcc nodejs_22 just ];
      };
    });
  };
}
