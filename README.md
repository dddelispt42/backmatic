# Backmatic

Backmatic is a Rust CLI and library that runs backup jobs from a single YAML configuration file. It orchestrates **rsync**, **Borg**, **Restic**, and **database** dumps (MySQL and PostgreSQL) with type-priority parallel scheduling, optional LUKS `destmount`, remote `srcmount` staging, retention policies, healthchecks.io pings, retries, and continuous mode.

Only one instance may run at a time; an XDG runtime lock file prevents overlapping runs.

## Requirements

### Build

- Rust toolchain (edition 2021)

### Runtime tools

Install only what your config uses:

| Tool | Used by |
|------|---------|
| `rsync` | rsync backups, `srcmount` pull |
| `borg` | Borg backups |
| `restic` | Restic backups |
| `mysqldump` / `pg_dump` | database backups |
| `gzip` | database compression |
| `mount` / `umount` / `cryptsetup` | optional `destmount` (LUKS) |
| `sshfs` | required for `srcmount` on **borg/restic** (FUSE mount of remote sources; rsync pulls over SSH directly) |
| `cp` / `rm` | rsync hard-link retention |

## Installation

```bash
cargo build --release
# binary: target/release/backmatic
```

Or use Docker:

```bash
just docker-build
```

## Usage

```bash
backmatic [OPTIONS]
```

| Option | Short | Default | Description |
|--------|-------|---------|-------------|
| `--configfile` | `-c` | `$XDG_CONFIG_HOME/backmatic/backmatic.yml` | Path to YAML config (`BACKMATIC_CONFIG`) |
| `--threads` | `-t` | `num_cpus / 2` | Worker threads (type-priority pool) |
| `--retryinterval` | `-i` | `3600` | Seconds between retry attempts |
| `--retries` | `-r` | `23` | Maximum retry attempts per job |
| `--continuous` | `-C` | `0` | Hours between cycle starts; `0` = run once |
| `--dry-run` | | off | Log planned jobs without executing |
| `--verbose` | `-v` | warn | `-v` info, `-vv` debug |

### Examples

```bash
# Validate config and show planned jobs
backmatic -c examples/minimal.yml --dry-run -v

# Run once with custom config
BACKMATIC_CONFIG=./backmatic.yml backmatic -v
```

## Configuration

Backmatic reads one YAML document. The file is validated against `schema/backmatic.schema.json` at load time, then semantically validated (e.g. `dest` must not be inside `src`).

```yaml
version: 1
defaults:
  logdir: /var/log/backmatic
  tmp_dir: /var/tmp/backmatic-tmp   # optional; exported as TMPDIR to restic/borg

rsync:    # optional
  - comment: "Documents"
    src: [/home/user/Documents/]
    dest: [/backup/documents/]

borg:     # optional
  - ...

restic:   # optional
  - ...

database: # optional — mysql or postgres per job
  - ...
```

### Common job fields

| Field | Required | Description |
|-------|----------|-------------|
| `comment` | recommended | Label for logs and archive names |
| `logdir` | no | Per-job log directory (default: `defaults.logdir`) |
| `src` | yes* | Local source path(s) |
| `srcmount` | yes* | Remote sources (structured entries; see below) |
| `dest` | yes** | Destination path(s) or repository URI(s) |
| `destmount` | yes** | LUKS block devices to mount before backup |
| `exclude` | no | Rsync/borg/restic exclude patterns |
| `password` | no | Borg/Restic repo password or LUKS passphrase |
| `keep_hourly` … `keep_yearly` | no | Retention (`0` = disabled) |
| `healthcheck` | no | healthchecks.io `{ url, uuid }` |

\* At least one of `src` or `srcmount` is required.  
\** At least one of `dest` or `destmount` is required. `dest` and `destmount` are independent lists — both are used when present.

`src`, `dest`, and `exclude` accept a string or YAML list (nested lists are flattened).

### `srcmount` (remote sources)

Structured entries only (no URI shorthand). The transport depends on the backup tool:

- **`borg` / `restic`** can only read local paths, so a remote `srcmount` is **mounted read-only via sshfs** (FUSE) into a local mount point. Files are read over the network on demand — local disk is only used for the backup destination, never a full remote mirror.
- **`rsync`** speaks SSH natively, so a remote `srcmount` is pulled directly with `rsync -e ssh user@host:path`. **No sshfs mount, no FUSE, no staging directory** is created for rsync jobs.

Authentication is **always key-based** (`identity_file`, the SSH agent, or `~/.ssh/config`); passwords are intentionally not supported for srcmount.

```yaml
srcmount:
  - host: backup.example
    port: 22
    user: backup
    path: /data
    identity_file: /keys/id_ed25519
    staging_dir: /var/tmp/backmatic-staging   # optional sshfs mount base (borg/restic only)
    ssh_options: ["StrictHostKeyChecking=accept-new"]  # optional extra transport -o options
```

For borg/restic, the default mount base is `$XDG_RUNTIME_DIR/backmatic-staging/{job-scope}/{origin_slug}/` (falls back to `/var/tmp/backmatic-staging`). Each job gets an isolated scope (`borg-1`, `restic-0`, …) so parallel jobs never share a mount point, and each source is mounted under a namespaced path (`sshfs_{host}_{path}`) so multi-origin backups stay distinguishable in Borg archive names and Restic tags. (`staging_dir` and the sshfs mount are ignored for rsync, which streams directly over SSH.)

**rsync destination layout with multiple sources.** rsync runs one independent `--delete` pass per source, so a job with **two or more** sources (`src` + `srcmount` combined) writes each source into its own slug subdirectory of the destination (`{dest}/{origin_slug}/`) — otherwise a later source's `--delete` would wipe an earlier source's files from the shared root. A **single-source** job keeps the classic flat layout, mirroring straight into `dest`. Note that adding a second source to an existing single-source job therefore relocates the first source under a slug subdirectory.

The sshfs mount uses `-o idmap=user` and deliberately **omits** `default_permissions`, so a non-root backmatic can read every file its SSH login can (permissions are enforced by the remote SSH server, not the local kernel). `ssh_options` are passed as extra `-o` options to the transport (sshfs for borg/restic, ssh for rsync). Running as root is only required when a job uses `destmount`.

**Non-interactive transport.** backmatic runs unattended, so every srcmount transport is forced non-interactive: `BatchMode=yes` (never prompt for a password or key passphrase — it fails fast instead), `ConnectTimeout=30` (give up quickly on an unreachable host), and `StrictHostKeyChecking=accept-new` (auto-trust a new host on first contact, reject a *changed* key). You can override the host-key policy by setting your own `StrictHostKeyChecking=...` in `ssh_options` (e.g. `yes` if you pre-seed `known_hosts`); `BatchMode`/`ConnectTimeout` are always enforced. These bounds apply only to *connection setup* — an authenticated session may run as long as a large or slow transfer needs, so long-running backups are never interrupted.

### `destmount` (encrypted destinations)

```yaml
destmount:
  - uuid: "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
    mountpoint: /mnt/backup        # default: /mnt/backapp/<uuid>
    password: "luks-passphrase"    # optional LUKS open
    path: "."                      # subpath under mountpoint (default: `.`)
```

Devices are unmounted when the job finishes.

### Rsync

- Command: `rsync -avHAXhE --delete --delete-excluded`
- Exit codes `23` and `24` are treated as success
- Local retention: hard-linked snapshots `{dest}.hourly-YY-MM-DD-HH-MM`, etc.

### Borg

- Auto `borg init` with `repokey` (when `password` set) or `none`
- Archives: `{dest}::{comment}-{origin_slug} {user}@{hostname}_{timestamp}`
- `borg prune` using `keep_*` values

### Restic

- Auto `restic init` when repo is missing
- Tags: `origin:{origin_slug}` per source
- Always runs `forget` then `prune` after backup

### Temp/scratch space (`TMPDIR`)

restic (and borg) buffer large temporary files while packing data — restic writes `restic-temp-pack-*` files to `$TMPDIR`. If `/tmp` is a small tmpfs, big backups fail with `no space left on device`. backmatic therefore runs these tools with `TMPDIR` pointed at `defaults.tmp_dir` (or `$BACKMATIC_TMPDIR`), defaulting to the disk-backed `/var/tmp/backmatic-tmp`. Point `tmp_dir` at a filesystem with enough free space (ideally on the same volume as, or as roomy as, your backup data).

### Database

```yaml
database:
  - comment: "All MySQL"
    engine: mysql          # mysql | postgres
    host: localhost
    user: backup
    password: secret
    src: []                # empty → all databases (mysql) or single dump
    dest: [/backup/sql/]
```

## Scheduling

Within each cycle, jobs are dispatched by **type priority**: rsync → borg → restic → database. Worker threads pick the highest-priority pending job; when rsync queues are empty, threads borrow to borg, then restic, and so on.

In **continuous mode** (`-C N`), cycle boundaries are anchored to wall clock. If a cycle overruns its slot, only jobs that **completed** in the prior cycle are re-queued; running and retrying jobs are not duplicated.

## Healthchecks

On success, Backmatic pings `POST {url}/ping/{uuid}`.

After all retries are exhausted, it pings `POST {url}/ping/{uuid}/fail` with a plain-text body containing `job_type`, `comment`, `attempts`, `last_error`, `logfile`, and `dest`.

## Project layout

```
src/
  lib.rs, main.rs, app.rs, cli.rs
  config/       YAML types, schema, validation, env ${VAR} resolution
  inject/       Clock, CommandExecutor, HttpClient, tool paths (DI)
  mount/        srcmount staging, destmount LUKS, origin slugs
  scheduler/    type-priority pool, continuous cycle
  runners/      rsync, borg, restic, database
  healthcheck/  healthchecks.io client
  retention/    rsync snapshot math
schema/backmatic.schema.json
examples/       sample configs
docs/           TESTING.md, EDITOR.md
```

## Testing

```bash
cargo test                                    # unit tests
cargo test --features integration-tests       # rsync/borg/restic (real binaries)
just test
```

See [docs/TESTING.md](docs/TESTING.md) for the full scenario matrix (T1–T24) and optional LUKS/SSH/DB features.

Editor YAML completion: [docs/EDITOR.md](docs/EDITOR.md).

## License

MIT — Heiko Riemer.
