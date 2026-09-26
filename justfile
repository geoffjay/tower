default:
    @just --list

# build everything
build:
    cargo build --workspace

# dev build + run tests
dev:
    cargo build --workspace && cargo test --workspace

# format, clippy, tests
lint:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

# run the server (dev)
run:
    cargo run -p tower-server -- serve

# end-to-end smoke (requires local herdr + pi)
e2e:
    ./scripts/e2e.sh