# Cross-compilation for the release artifacts.
#
# The artifacts are statically linked against musl because the runners consuming
# them are frequently Alpine-based, where a glibc-linked binary dies with a loader
# error that says nothing useful.
#
# rustls pulls in `ring`, so unlike a pure-Rust crate both targets need a full C
# compiler and not merely a linker; that is why CC_* is set alongside the linker.
{ ... }:
{
  perSystem =
    { pkgs, lib, ... }:
    let
      crossCc = {
        "x86_64-unknown-linux-musl" = pkgs.pkgsCross.musl64.stdenv.cc;
        "aarch64-unknown-linux-musl" = pkgs.pkgsCross.aarch64-multiplatform-musl.stdenv.cc;
      };

      underscore = builtins.replaceStrings [ "-" ] [ "_" ];
      ccPath = triple: "${crossCc.${triple}}/bin/${triple}-cc";

      # The `cc` crate accepts CC_<triple> with dashes too, but a dashed name is not
      # a valid shell identifier and devshell exports these.
      envFor = triple: [
        {
          name = "CARGO_TARGET_${lib.toUpper (underscore triple)}_LINKER";
          value = ccPath triple;
        }
        {
          name = "CC_${underscore triple}";
          value = ccPath triple;
        }
      ];
    in
    {
      devshells.default = {
        packages = builtins.attrValues crossCc;
        env = lib.concatMap envFor (builtins.attrNames crossCc);
      };

      # Exposed so CI can resolve the cross compilers without entering the devshell.
      packages.musl-cross-cc = pkgs.linkFarm "musl-cross-cc" (
        lib.mapAttrsToList (name: path: { inherit name path; }) crossCc
      );
    };
}
