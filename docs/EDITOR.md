# Editor support

Backmatic configuration files are YAML documents validated against a JSON Schema.

## Schema location

```
schema/backmatic.schema.json
```

The schema covers `version`, job lists (`rsync`, `borg`, `restic`, `database`), `srcmount`, `destmount`, retention fields, and `healthcheck`.

## Visual Studio Code

Install the [YAML extension](https://marketplace.visualstudio.com/items?itemName=redhat.vscode-yaml) and add to `.vscode/settings.json` (workspace or user):

```json
{
  "yaml.schemas": {
    "./schema/backmatic.schema.json": [
      "backmatic.yml",
      "**/backmatic.yml",
      "**/.config/backmatic/*.yml"
    ]
  }
}
```

Use a repository-relative path so the schema resolves when the workspace root is the Backmatic checkout.

## Other editors

Point your YAML language server at `schema/backmatic.schema.json` using the same file globs. JetBrains IDEs support custom JSON Schema mappings under **Languages & Frameworks → Schemas and DTDs**.

## Validate from the CLI

Schema validation runs automatically when loading config. To check a file without running backups:

```bash
cargo run -- -c examples/minimal.yml --dry-run -v
```

Invalid YAML or schema violations produce a clear error before any job starts.
