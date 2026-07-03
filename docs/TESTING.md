# Testing Backmatic

## Unit tests

Unit tests run without external backup tools:

```bash
cargo test
# or
just test-unit
```

Coverage includes config parsing, JSON Schema validation, semantic validation, scheduler overrun behavior, origin slug generation, and mocked healthcheck HTTP.

## Integration tests

Integration tests exercise real `rsync`, `borg`, and `restic` binaries. Enable the feature and run sequentially (repos share process-global state in some cases):

```bash
cargo test --features integration-tests -- --test-threads=1
```

### Requirements

| Tool | Debian/Ubuntu package |
|------|------------------------|
| rsync | `rsync` |
| Borg | `borgbackup` |
| Restic | `restic` |

Optional scenarios (marked `#[ignore]` in `tests/integration_misc.rs`):

| Feature | Scenario | Requirements |
|---------|----------|--------------|
| `integration-luks` | T15, T24 LUKS `destmount` | root/`CAP_SYS_ADMIN`, `cryptsetup`, loop device |
| `integration-ssh` | T14 `srcmount` pull | OpenSSH server or testcontainer, `sshfs` |
| `integration-db` | T16–T17 database dumps | MySQL/Postgres testcontainers |

### Scenario matrix (T1–T24)

| ID | rsync | borg | restic | Notes |
|----|-------|------|--------|-------|
| T1–T7 | yes | yes | partial | restore/diff via snapshots |
| T8–T9 | unit + partial | yes | T22 | retention uses injectable `Clock` in unit tests |
| T10–T12 | yes | yes | yes | exclude patterns |
| T13 | unit | unit | unit | `validate.rs` |
| T14–T17 | — | — | — | ignored; optional CI nightly |
| T18 | misc | misc | misc | mock HTTP |
| T20 | unit + misc | | | overrun tick |
| T22 | — | — | yes | forget + prune |
| T23 | — | yes | yes | multi-origin attribution |

Test data is generated at runtime in `tempfile::TempDir` via `tests/common::TestTree::basic()` — nothing large is committed to git.

## LUKS integration (manual)

1. Create a loop-backed LUKS volume (requires privileges).
2. Add a `destmount` entry with the volume UUID to a test config.
3. Run `cargo test --features integration-luks -- --ignored`.

See `PLAN.md` section 7.6 for the full outline.

## CI

- **`.github/workflows/ci.yml`** — fmt, clippy, unit tests
- **`.github/workflows/integration.yml`** — installs borg/restic/rsync, runs integration tests

## Docker

```bash
just docker-build
just docker-run -- --dry-run -v
```

Mount your config and data volumes as documented in the `Justfile`.
