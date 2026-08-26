{ ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      packages.forgejo-release = pkgs.writeShellApplication {
        name = "forgejo-release";
        runtimeInputs = [
          pkgs.curl
          pkgs.jq
        ];
        text = ''
          if [ "$#" -ne 2 ]; then
            echo "usage: forgejo-release <tag> <asset-dir>" >&2
            echo "env: FORGEJO_URL, FORGEJO_REPO (owner/repo), FORGEJO_TOKEN" >&2
            exit 2
          fi

          tag="$1"
          assets="$2"
          api="''${FORGEJO_URL:?}/api/v1/repos/''${FORGEJO_REPO:?}"

          call() {
            local method="$1"
            shift
            curl --fail-with-body --silent --show-error \
              -X "$method" \
              -H "Authorization: token ''${FORGEJO_TOKEN:?}" \
              -H "Accept: application/json" \
              "$@"
          }

          # Re-runnable: adopt an existing release for the tag rather than
          # failing, so a partially uploaded release can be completed.
          if release=$(call GET "$api/releases/tags/$tag" 2>/dev/null); then
            echo "release $tag already exists, updating assets"
          else
            release=$(call POST "$api/releases" \
              -H "Content-Type: application/json" \
              --data "$(jq -n --arg tag "$tag" '{
                tag_name: $tag,
                name: $tag,
                draft: false,
                prerelease: false,
                body: "Statically linked musl binaries. Each tarball has a matching .sha256 asset."
              }')")
            echo "created release $tag"
          fi

          id=$(jq -r .id <<<"$release")

          for file in "$assets"/*; do
            name=$(basename "$file")

            # Uploading a duplicate name would silently create a second asset.
            old=$(call GET "$api/releases/$id/assets" |
              jq -r --arg name "$name" '.[] | select(.name == $name) | .id')
            for asset_id in $old; do
              call DELETE "$api/releases/$id/assets/$asset_id" >/dev/null
            done

            call POST "$api/releases/$id/assets?name=$name" \
              -F "attachment=@$file" >/dev/null
            echo "uploaded $name"
          done
        '';
      };
    };
}
