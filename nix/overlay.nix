{ inputs, ... }:
{
  flake.overlays.default = final: prev: {
    helm-vendor = inputs.self.packages.${final.stdenv.hostPlatform.system}.default;
  };
}
