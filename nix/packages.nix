{ inputs, self, ... }:
{
  perSystem =
    { system, pkgs, ... }:
    let
      cargoNix = inputs.crate2nix.tools.${system}.appliedCargoNix {
        name = "helm-vendor";
        src = self;
      };

      helm-vendor-unwrapped = cargoNix.workspaceMembers.helm-vendor.build;

      helm-vendor-wrapped = pkgs.symlinkJoin {
        name = "helm-vendor";
        paths = [ helm-vendor-unwrapped ];
        buildInputs = [ pkgs.makeWrapper ];
        postBuild = ''
          wrapProgram $out/bin/helm-vendor \
            --prefix PATH : ${pkgs.lib.makeBinPath [ pkgs.kubernetes-helm pkgs.diffutils ]}
        '';
      };
    in
    {
      checks.helm-vendor = helm-vendor-unwrapped;

      packages = {
        default = helm-vendor-wrapped;
      };

      apps.default = {
        type = "app";
        program = "${helm-vendor-wrapped}/bin/helm-vendor";
      };
    };
}
