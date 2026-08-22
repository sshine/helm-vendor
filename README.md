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
  add         Declare a chart in charts.toml and vendor it. Creates charts.toml if it does not exist
  remove      Undeclare charts and delete their vendored directories
  sync        Vendor charts so charts/ matches charts.toml. Defaults to every chart
  update      Re-pin charts to the latest upstream version and vendor them
  list        List charts and their pinned versions
  completion  Print a shell completion script. Source it with e.g. `eval "$(helm-vendor completion zsh)"`
  help        Print this message or the help of the given subcommand(s)

Options:
      --config <CONFIG>  Path to charts.toml. Auto-detected by walking up from CWD if omitted
  -h, --help             Print help
  -V, --version          Print version
```

Every chart-selecting command takes zero or more chart names, where naming none means all of
them. So `sync` vendors the whole registry and `sync cilium cert-manager` vendors two charts.

#### Adding a new chart

Omit the version to pin whatever upstream currently offers.

```console
$ helm-vendor add cert-manager https://charts.jetstack.io/
  added cert-manager 1.21.1

$ git add charts.toml charts/cert-manager/
$ git commit -m "feat(charts): Vendored cert-manager 1.21.1"
```

#### Updating charts to their latest upstream versions

`update` re-pins and vendors in one step, so charts.toml and `charts/` never disagree.

```console
$ helm-vendor update
  cert-manager v1.20.1 -> v1.21.1
  updated cert-manager
synced 1 of 1 chart(s)

$ git add charts.toml charts/cert-manager/
$ git commit -m "chore(charts): Updated cert-manager to v1.21.1"
```

To see what is available before committing to it, ask `list`:

```console
$ helm-vendor list --outdated
NAME          VERSION   LATEST     source
cert-manager  v1.20.1   v1.21.1 ↑  https://charts.jetstack.io/
```

#### Removing a chart

```console
$ helm-vendor remove cert-manager
  removed cert-manager
```

Pass `--keep-files` to drop the charts.toml entry but leave the vendored directory behind.
`add` and `update` take the converse, `--no-fetch`, to write charts.toml without downloading.

#### Verifying vendored charts in CI

`sync --check` pulls every chart and compares it against what is committed, exiting non-zero
when they differ. `list --outdated` exits non-zero when an upgrade is available.

```console
$ helm-vendor sync --check
18 chart(s) in sync
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
