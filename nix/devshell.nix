{ inputs, ... }:
{
  imports = [ inputs.devshell.flakeModule ];
  perSystem =
    { config, pkgs, ... }:
    let
      rust-toolchain = pkgs.rust-bin.fromRustupToolchainFile ../rust-toolchain.toml;
    in
    {
      devshells.default = {
        packages = [
          rust-toolchain
          config.treefmt.build.wrapper
          config.hk-nix.package
          pkgs.cargo-readme
          pkgs.deadnix
          pkgs.diffutils
          pkgs.git
          pkgs.just
          pkgs.kubernetes-helm
          pkgs.stdenv.cc
        ];

        env = [
          {
            name = "RUST_BACKTRACE";
            value = "1";
          }
        ];

        devshell.motd = "";
        devshell.startup.hk.text = config.hk-nix.shellHook;
      };
    };
}
