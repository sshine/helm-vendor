## helm-vendor

CLI for vendoring Helm charts from a declarative `charts.toml` registry.

Charts are declared in a `charts.toml` file and vendored into a sibling
`charts/` directory. `charts.toml` is auto-detected by walking up from the
current directory; override it with `--config <path>`.

### Subcommands

- `add <name> <url> [version]` — Add a new chart to `charts.toml` and vendor
  it, creating `charts.toml` if it does not exist. The URL is an `https://`
  Helm repo or an `oci://` reference; omit `version` to pin the latest
  upstream release. If the chart already exists this behaves like `update`
  when the source matches, and fails on a source conflict.
- `fetch [name] [--all]` (alias `vendor`) — Pull chart(s) from upstream into
  the `charts/` directory.
- `check [name] [--all]` — Verify vendored charts match their upstream
  source.
- `update <name> <version>` / `update <name> --latest` /
  `update --all --latest` — Update the pinned version of a chart (or every
  chart) in `charts.toml`.
- `list [--check]` — List all charts and their versions; `--check` adds a
  `LATEST` column resolved from upstream.
- `outdated [name]` — Show charts whose pinned version is older than what's
  upstream.
- `completion <shell>` — Print a shell completion script for bash, zsh, or
  fish.
