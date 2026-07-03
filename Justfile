default:
    @just --list

build:
    cargo build --release

test:
    cargo test

test-unit:
    cargo test --lib

test-integration:
    for t in integration_rsync integration_borg integration_restic integration_misc; do cargo test --features integration-tests --test "$t" -- --test-threads=1; done

test-all:
    for t in integration_rsync integration_borg integration_restic integration_misc; do cargo test --features integration-tests --test "$t" -- --test-threads=1; done

clippy:
    cargo clippy --all-targets -- -D warnings

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all -- --check

audit:
    cargo install cargo-audit --locked 2>/dev/null || true
    cargo audit

docker-build:
    docker build -f docker/Dockerfile -t backmatic:latest .

docker-run *ARGS:
    docker run --rm \
      -v "${BACKMATIC_CONFIG:-./examples/backmatic.yml}:/config/backmatic.yml:ro" \
      -v "${BACKMATIC_SRC:-./data/src}:/data/src:ro" \
      -v "${BACKMATIC_DEST:-./data/dest}:/data/dest" \
      -v "${BACKMATIC_KEYS:-./keys}:/keys:ro" \
      backmatic:latest {{ARGS}}
