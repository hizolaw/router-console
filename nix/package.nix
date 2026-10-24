{ lib, rustPlatform, buildNpmPackage, nodejs_22 }:
let
  version = (builtins.fromTOML (builtins.readFile ../Cargo.toml)).package.version;
  frontend = buildNpmPackage {
    pname = "router-console-web";
    inherit version;
    src = lib.fileset.toSource {
      root = ../web;
      fileset = lib.fileset.unions [
        ../web/package.json ../web/package-lock.json ../web/index.html
        ../web/tsconfig.json ../web/vite.config.ts ../web/src ../web/public
      ];
    };
    nodejs = nodejs_22;
    npmDepsHash = "sha256-s2JMdkHmoemruRJ2Sg/whNwppRSsj9Keb6wdm4zbrQM=";
    installPhase = ''
      runHook preInstall
      mkdir -p $out
      cp -r dist/. $out/
      runHook postInstall
    '';
  };
in rustPlatform.buildRustPackage {
  pname = "router-console";
  inherit version;
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [ ../Cargo.toml ../Cargo.lock ../build.rs ../src ];
  };
  cargoLock.lockFile = ../Cargo.lock;
  preBuild = ''
    mkdir -p web/dist
    cp -r ${frontend}/. web/dist/
  '';
  meta = {
    description = "Lightweight modular Linux router console";
    homepage = "https://github.com/hizolaw/router-console";
    mainProgram = "router-console";
    platforms = [ "aarch64-linux" "x86_64-linux" ];
  };
}
