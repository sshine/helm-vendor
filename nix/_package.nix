# The build definition, shared by this flake's packages and by the overlay.
#
# Taking `pkgs` as an argument (rather than closing over this flake's own) is what
# lets the overlay build against the consumer's nixpkgs, so downstream can override
# and cross-compile it.
{
  lib,
  rustPlatform,
  makeWrapper,
  kubernetes-helm,
  diffutils,
  ...
}:
rustPlatform.buildRustPackage {
  pname = "helm-vendor";
  version = (lib.importTOML ../Cargo.toml).package.version;

  # Naming the inputs explicitly keeps target/ and .direnv/ out of the store, and
  # means an unrelated edit does not invalidate the build.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../src
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;

  nativeBuildInputs = [ makeWrapper ];

  # helm shells out for every pull and diff drives `check`; neither is something a
  # Nix-installed binary may assume exists on the user's PATH.
  postInstall = ''
    wrapProgram $out/bin/helm-vendor \
      --prefix PATH : ${
        lib.makeBinPath [
          kubernetes-helm
          diffutils
        ]
      }
  '';

  meta = {
    description = "CLI for vendoring Helm charts from a charts.toml registry";
    mainProgram = "helm-vendor";
    license = with lib.licenses; [
      mit
      asl20
    ];
  };
}
