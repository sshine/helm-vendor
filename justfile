_default:
    @just --list --unsorted

fmt-check:
    nix develop -c treefmt --fail-on-change

lint:
    nix develop -c cargo clippy -- -D warnings

test:
    nix develop -c cargo test

build:
    nix develop -c cargo build

doc:
    nix develop -c cargo doc --no-deps

readme-check:
    nix develop -c cargo-readme-check --check

package-check:
    nix develop -c cargo package --workspace

lock-check:
    nix develop -c cargo metadata --locked --format-version 1 > /dev/null

ci: fmt-check lock-check lint test build doc readme-check package-check

# Demo `list --check` rendering with fake data, parallel resolution, and color.
# Models the real binary's behavior: counter ticks out-of-order as charts
# "complete" (random delay), but rows print in registry order.
simulate:
    #!/usr/bin/env bash
    set -euo pipefail

    charts=(
        "alloy|1.7.0|1.8.0|https://grafana.github.io/helm-charts"
        "argo-cd|7.8.4|9.5.10|oci://ghcr.io/argoproj/argo-helm/argo-cd"
        "cert-manager|v1.20.1|v1.20.2|https://charts.jetstack.io/"
        "cert-manager-webhook-hetzner|0.6.7|0.7.0|https://charts.hetzner.cloud/"
        "cilium|1.19.2|1.19.3|https://helm.cilium.io/"
        "cloudnative-pg|0.28.0|0.28.0|https://cloudnative-pg.github.io/charts"
        "external-dns|1.20.0|1.21.1|https://kubernetes-sigs.github.io/external-dns/"
        "forgejo|16.2.2|17.0.1|oci://code.forgejo.org/forgejo-helm/forgejo"
        "harbor|1.18.3|1.18.3|https://helm.goharbor.io"
        "hcloud-cloud-controller-manager|1.30.1|1.30.1|https://charts.hetzner.cloud"
        "hcloud-csi|2.20.0|2.20.2|https://charts.hetzner.cloud"
        "keycloakx|7.1.9|7.1.11|https://codecentric.github.io/helm-charts"
        "kube-prometheus-stack|69.7.4|84.4.0|https://prometheus-community.github.io/helm-charts"
        "loki|6.25.0|7.0.0|https://grafana.github.io/helm-charts"
        "tempo|1.18.1|1.24.4|https://grafana.github.io/helm-charts"
        "trust-manager|v0.22.0|v0.22.1|https://charts.jetstack.io/"
        "vault|0.32.0|0.32.0|https://helm.releases.hashicorp.com"
        "vault-secrets-operator|1.3.0|1.3.0|https://helm.releases.hashicorp.com"
    )

    total=${#charts[@]}
    name_w=31
    lv=7
    lw_uptodate=$((lv + 2))
    lw_outdated=$((lv + 4))

    interactive=0
    color_on=0
    [[ -t 1 && -t 2 ]] && interactive=1
    [[ -t 1 && -z "${NO_COLOR:-}" ]] && color_on=1

    # $'…' embeds real ESC bytes so `printf '%s'` (no %b) prints them.
    green=$'\033[32m'
    yellow=$'\033[33m'
    orange=$'\033[38;5;208m'
    reset=$'\033[0m'

    severity() {
        local pinned=${1#v} latest=${2#v}
        local pmaj pmin _prest lmaj lmin _lrest
        IFS='.' read -r pmaj pmin _prest <<<"$pinned"
        IFS='.' read -r lmaj lmin _lrest <<<"$latest"
        if [[ "$pmaj" != "$lmaj" ]]; then echo major
        elif [[ "$pmin" != "$lmin" ]]; then echo minor
        else echo patch
        fi
    }

    paint_for() {
        case "$1" in
            patch) printf '%s' "$green" ;;
            minor) printf '%s' "$yellow" ;;
            major) printf '%s' "$orange" ;;
        esac
    }

    header=$(printf '%-*s  %-8s  %-*s  %s' "$name_w" "NAME" "VERSION" "$lw_uptodate" "LATEST" "source")

    # Assign each chart a random "completion delay" (10–600ms). Sorting these
    # gives us the order in which the parallel resolver would observe results.
    declare -a delays
    for i in "${!charts[@]}"; do
        delays[i]=$((10 + RANDOM % 590))
    done

    # completion_order[k] = chart index that finishes k-th
    mapfile -t completion_order < <(
        for i in "${!delays[@]}"; do printf '%d %d\n' "${delays[$i]}" "$i"; done | sort -n | awk '{print $2}'
    )

    if (( ! interactive )); then
        printf '%s\n' "$header"
    fi

    declare -a results=()
    for i in "${!charts[@]}"; do results[i]=""; done

    rendered=()
    next_emit=0
    completed=0
    prev_ms=0
    for sidx in "${completion_order[@]}"; do
        cur_ms=${delays[$sidx]}
        sleep_ms=$((cur_ms - prev_ms))
        if (( sleep_ms > 0 )); then
            sleep "$(printf '0.%03d' "$sleep_ms")"
        fi
        prev_ms=$cur_ms

        completed=$((completed + 1))
        results[sidx]=ready

        IFS='|' read -r cname _ _ _ <<<"${charts[$sidx]}"
        if (( interactive )); then
            printf '\r\x1b[2K[%d/%d] %s' "$completed" "$total" "$cname" >&2
        fi

        # Flush the in-order prefix of newly-ready slots.
        while (( next_emit < total )) && [[ -n "${results[$next_emit]}" ]]; do
            IFS='|' read -r name version latest source <<<"${charts[$next_emit]}"
            latest_part=$(printf '%-*s' "$lv" "$latest")
            if [[ "$version" != "$latest" ]]; then
                inner="$latest_part ↑"
                lw=$lw_outdated
                if (( color_on )); then
                    sev=$(severity "$version" "$latest")
                    color=$(paint_for "$sev")
                    cell="${color}${inner}${reset}"
                    # Color escapes don't widen the cell visually; pad the
                    # printable portion to lw, then re-wrap so column widths
                    # line up across rows.
                    printable=$(printf '%-*s' "$lw" "$inner")
                    cell="${color}${printable}${reset}"
                else
                    cell=$(printf '%-*s' "$lw" "$inner")
                fi
            else
                cell=$(printf '%-*s' "$lw_uptodate" "$latest_part  ")
            fi
            row=$(printf '%-*s  %-8s  %s  %s' "$name_w" "$name" "$version" "$cell" "$source")
            if (( interactive )); then
                rendered+=("$row")
            else
                printf '%s\n' "$row"
            fi
            next_emit=$((next_emit + 1))
        done
    done

    if (( interactive )); then
        printf '\r\x1b[2K' >&2
        printf '%s\n' "$header"
        for row in "${rendered[@]}"; do
            printf '%s\n' "$row"
        done
    fi
