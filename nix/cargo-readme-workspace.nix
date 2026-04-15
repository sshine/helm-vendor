{ ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      packages.cargo-readme-workspace = pkgs.writeShellScriptBin "cargo-readme-workspace" ''
        set -euo pipefail

        check=0
        if [ "$#" -gt 0 ] && [ "$1" = "--check" ]; then
          check=1
        fi

        root="$(git rev-parse --show-toplevel)"
        tmp=$(mktemp)
        trap 'rm -f "$tmp"' EXIT
        (cd "$root" && cargo readme --no-title --no-license --no-badges) > "$tmp"

        if ! diff -q "$tmp" "$root/README.md" >/dev/null 2>&1; then
          if [ "$check" -eq 1 ]; then
            diff "$tmp" "$root/README.md" || true
            rm -f "$tmp"
                echo "README.md is out of date. Run 'cargo-readme-workspace' and commit the result." >&2
            exit 1
          fi
          mv "$tmp" "$root/README.md"
          echo "README.md updated."
        else
          rm -f "$tmp"
          echo "README.md is up to date."
        fi
      '';
    };
}
