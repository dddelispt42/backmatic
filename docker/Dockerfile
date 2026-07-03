# syntax=docker/dockerfile:1

FROM rust:bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    rsync borgbackup restic openssh-client sshfs cryptsetup \
    default-mysql-client postgresql-client gzip mount util-linux ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/backmatic /usr/local/bin/backmatic
VOLUME ["/config", "/data/src", "/data/dest", "/keys"]
ENTRYPOINT ["/usr/local/bin/backmatic"]
CMD ["-c", "/config/backmatic.yml"]
