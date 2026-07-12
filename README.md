## helm-vendor

CLI for vendoring Helm charts from a declarative `charts.toml` registry.

Charts are declared in a `charts.toml` file and vendored into a sibling `charts/` directory.

`charts.toml` is auto-detected by walking up from the current directory; override with `--config <path>`.

### Usage

```
$ helm-vendor --help
Vendor Helm charts from a declarative charts.toml registry

Usage: helm-vendor [OPTIONS] <COMMAND>

Commands:
  add         Add a new chart to charts.toml and vendor it. Creates charts.toml if it does not exist
  fetch       Pull chart(s) from upstream into the charts/ directory
  check       Verify vendored charts match their upstream source
  update      Update the version of a chart in charts.toml
  list        List all charts and their versions
  outdated    Show charts whose pinned version is older than what's upstream
  completion  Print a shell completion script. Source it with e.g. `eval "$(helm-vendor completion zsh)"`
  help        Print this message or the help of the given subcommand(s)

Options:
      --config <CONFIG>  Path to charts.toml. Auto-detected by walking up from CWD if omitted
  -h, --help             Print help
  -V, --version          Print version
```

#### Adding a new chart

```console
$ helm-vendor add cert-manager https://charts.jetstack.io/
  added cert-manager 1.21.0

$ git add charts.toml charts/cert-manager/
$ git commit -m "feat(charts): Vendored cert-manager 1.21.0"
```

#### Updating all charts to their latest upstream versions

```console
$ helm-vendor update --all --latest
  updated cert-manager to 1.21.0

$ git add charts.toml charts/cert-manager/
$ git commit -m "chore(charts): Updated cert-manager to 1.21.0"
```

### Installation

For Nix/NixOS: Add `helm-vendor` to your flake inputs and include its overlay:

```nix
{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
  inputs.helm-vendor = {
    url = "git+https://git.shine.town/infra/helm-vendor.git";
    inputs.nixpkgs.follows = "nixpkgs";
  };

  outputs = { self, nixpkgs, helm-vendor, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs {
        inherit system;
        overlays = [ helm-vendor.overlays.default ];
      };
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = [
          pkgs.helm-vendor
          pkgs.kubernetes-helm
        ];
      };
    };
}
```

Or use it directly:

```console
$ nix run git+https://git.shine.town/infra/helm-vendor.git#helm-vendor -- --help
```
