{ ... }:
{
  perSystem =
    { pkgs, lib, ... }:
    rec {
      checks.helm-vendor = packages.default;

      packages.default = pkgs.callPackage ./_package.nix { };

      # The README advertises `nix run <url>#helm-vendor`.
      packages.helm-vendor = packages.default;

      apps.default = {
        type = "app";
        program = lib.getExe packages.default;
      };
    };
}
