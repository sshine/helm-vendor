# Prebuilt release binaries, for consumers who want helm-vendor without a Rust
# toolchain entering their build closure.
#
# The pinned version deliberately trails Cargo.toml: it names the last tag whose
# artifacts are published, not whatever the working tree happens to claim. Bump
# `release` here once a tag's Release workflow has finished.
#
# The hashes are the ones the release publishes alongside each tarball, so they
# can be checked against the .sha256 assets rather than taken on faith.
{ ... }:
{
  perSystem =
    {
      system,
      pkgs,
      lib,
      ...
    }:
    let
      release = {
        version = "0.6.0";
        hashes = {
          "x86_64-linux" = "sha256-3PGh0WMIjElm4UK5FS2g+DXUeuTH4/ZFQ0d2XzjssEM=";
          "aarch64-linux" = "sha256-0QA5PBR/6NHo9QFuU51WAP38KArT1iqpQkHZ/T4/SmE=";
        };
      };

      triple = {
        "x86_64-linux" = "x86_64-unknown-linux-musl";
        "aarch64-linux" = "aarch64-unknown-linux-musl";
      };

      tag = "v${release.version}";

      tarball = pkgs.fetchurl {
        url = "https://git.shine.town/infra/helm-vendor/releases/download/${tag}/helm-vendor-${tag}-${triple.${system}}.tar.gz";
        hash = release.hashes.${system};
      };
    in
    lib.optionalAttrs (release.hashes ? ${system}) {
      packages.helm-vendor-bin =
        pkgs.runCommand "helm-vendor-bin-${release.version}"
          {
            nativeBuildInputs = [ pkgs.makeWrapper ];

            meta = {
              description = "CLI for vendoring Helm charts from a charts.toml registry (prebuilt)";
              mainProgram = "helm-vendor";
              license = with lib.licenses; [
                mit
                asl20
              ];
              platforms = lib.attrNames release.hashes;
              sourceProvenance = [ lib.sourceTypes.binaryNativeCode ];
            };
          }
          ''
            tar -xzf ${tarball}
            install -Dm755 helm-vendor $out/bin/helm-vendor

            wrapProgram $out/bin/helm-vendor \
              --prefix PATH : ${
                lib.makeBinPath [
                  pkgs.kubernetes-helm
                  pkgs.diffutils
                ]
              }
          '';
    };
}
