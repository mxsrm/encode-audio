# Justfile

fmt:
    cargo +nightly fmt --all

clippy:
    cargo +nightly clippy

clean:
    git clean -dfx

build:
    cargo build --profile release
