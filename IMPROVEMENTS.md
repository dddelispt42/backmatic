# Backmatic — Suggested Improvements

This document captures structural and design improvements identified from a review of the codebase. Items are grouped by theme and ordered roughly by impact.

---

## Architecture & Modularity

### 1. Extract a shared backup runner trait

`borg.rs`, `restic.rs`, `rsync.rs`, and `database.rs` repeat the same pattern:

- Create a thread pool
- Iterate over a YAML list
- Build a `BackupConfig`
- Check that the external binary exists
- Optionally mount a destination device
- Run the backup

A small `BackupRunner` trait or shared dispatcher would reduce duplication and make adding new backends straightforward.

### 2. Split library vs binary

Move core logic into `lib.rs` (`Backmatic`, config parsing, runners) and keep `main.rs` thin. This enables integration tests and reuse without spawning a separate process.

### 3. Replace raw `Yaml` with typed config

Deserialize into `serde` structs (`RsyncJob`, `BorgJob`, `ResticJob`, `DatabaseJob`, etc.) instead of indexing `cfg["borg"]` everywhere. This enables validation at parse time and removes panics on unexpected YAML types.

### 4. Centralize external command execution

Wrap `std::process::Command` in a small module that handles logging, environment variables, stdout/stderr capture, and exit-code policy. Borg and Restic share nearly identical `is_repo_existing` / `init_repo` flows that could be unified.

---

## Configuration & Portability

### 5. Remove hardcoded paths

The default config path is `/home/heiko/.config/backmatic.yml` and the lock file is `/tmp/backmatic.lock`. Use XDG base directories (e.g. via the `dirs` crate): `~/.config/backmatic/config.yml` for config and a runtime directory for the lock file. A TODO for this already exists in `config.rs`.

### 6. Validate config with a schema

The code has TODOs for YAML schema validation. Add `serde` + `validator`, or validate against a JSON Schema, before running any jobs.

### 7. Support config via environment variable

Expose something like `BACKMATIC_CONFIG` through clap's `env` feature, alongside the existing `-c` / `--configfile` flag.

### 8. Improve secrets handling

Passwords in plain YAML are a security risk. Support environment variable references (e.g. `password: ${BORG_PASS}`) or a separate secrets file with restricted permissions.

---

## Error Handling & Reliability

### 9. Replace `expect` / `panic` with `Result`

Config loading, YAML parsing, and command failures often abort the entire process. Propagate errors with context using `anyhow` or `thiserror`.

### 10. Stop using `Result<(), ()>`

Borg and Restic helpers return `Result<(), ()>`, which discards failure reasons. Use a real error type so retries and logging can report what went wrong.

### 11. Fix retry off-by-one

Retry loops use `for _ in 1..retry_count` (e.g. 22 attempts when `--retries 23`). Use `0..retry_count` or `1..=retry_count` consistently and document the intended behavior.

### 12. Consistent mount lifecycle on failure

`Mounter` relies on `Drop` for unmount. Error paths across modules should use RAII consistently (e.g. a `MountGuard` type) so devices are always unmounted, including on partial failure.

---

## `mount.rs` Bugs & Design

### 13. Fix `cryptsetup` invocation

`Command::new("echo pw | cryptsetup ...")` treats the entire string as the executable name. Use `Command` with proper arguments, or document and isolate shell usage if `sh -c` is intentional (prefer explicit arguments).

### 14. Fix LUKS mount path after `luksOpen`

After `luksOpen`, the mount target should be `/dev/mapper/<uuid>`. The current code may still pass `self.device` to `mount` in some paths.

### 15. Fix `cryptsetup luksClose` invocation

The `umount` path has the same `Command::new` string bug as `luksOpen`.

### 16. Database jobs ignore `destmount`

Unlike rsync, Borg, and Restic, `database.rs` never mounts destinations. Either add `destmount` support or document the limitation explicitly (README already notes this).

---

## Dependencies & Maintenance

### 17. Replace `yaml-rust`

`yaml-rust` is unmaintained. Migrate to `serde_yaml`, which integrates with `serde` and is actively maintained.

### 18. Remove unused `command` dependency

`command = "0.0.0"` is listed in `Cargo.toml` but not used in the source.

### 19. Document concurrency model

`threadpool` is reasonable for subprocess I/O. Document why threads were chosen over alternatives, or unify on one concurrency approach if the codebase grows.

---

## Testing & CI

### 20. Replace placeholder tests

Many tests are `assert!(false)` or empty `should_panic` stubs. Add:

- Config parsing fixtures (valid and invalid YAML)
- Unit tests for `filenamify`, `yaml2string_list`, and retention helpers
- Integration tests with mocked or sandboxed commands (`assert_cmd` + temp directories)

### 21. Modernize CI

`.travis.yml` is the only CI config. GitHub Actions with `cargo test`, `clippy`, and `rustfmt` would match current Rust ecosystem practice.

---

## Observability & Operations

### 22. Structured logging

Optional JSON log output for cron/systemd consumption. Ensure every log line can be correlated with a job `comment`.

### 23. Dry-run / list mode

A `--dry-run` flag to print planned commands without executing them would help validate configuration before a real backup run.

### 24. Meaningful exit codes

Return a non-zero exit code if any job failed, instead of only logging warnings and exiting successfully.

---

## Feature Gaps

### 25. Support additional database backends

Only MySQL is supported via a hardcoded `/usr/bin/mysqldump` path. An extensible database driver enum (PostgreSQL, SQLite, etc.) would broaden usefulness.

### 26. Configurable binary paths

Paths like `/usr/bin/rsync`, `/usr/bin/borg`, and `/usr/bin/restic` are hardcoded. Allow overrides in YAML or via environment variables for non-FHS layouts.

### 27. Restic prune after forget

`restic forget` is run but `restic prune` is not. Document this behavior or add an optional prune step for space reclamation.

### 28. Notification hooks

On failure (or on completion), optionally invoke a webhook, email, or user-defined script—common for backup automation tools.

---

## Summary Priority Matrix

| Priority | Items |
|----------|-------|
| **High** (bugs / security) | 8, 13, 14, 15, 9 |
| **Medium** (maintainability) | 1, 3, 4, 5, 17, 20 |
| **Lower** (nice to have) | 22, 23, 25, 28 |
