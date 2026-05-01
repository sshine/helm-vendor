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
