# Justfile

fmt:
    cargo +nightly fmt --all

clippy:
    cargo +nightly clippy

clean:
    git clean -dfx

build:
    cargo build --profile release

release:
    cargo smart-release --no-push --no-changelog-preview --allow-dirty --no-publish --execute
