{ ... }:
{
  flake.overlays.default = final: _prev: {
    helm-vendor = final.callPackage ./_package.nix { };
  };
}
