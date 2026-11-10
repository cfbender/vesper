# Vesper configuration files

`vesper_core::config` reads one configuration directory (for example `/config`). YAML is the only authority; nothing is stored in the database.

## Layout

A directory is a mapping. Inside it:

- `_index.yaml` contributes keys of the directory's own mapping.
- `name.yaml` contributes the value of key `name`. The file does not repeat the key.
- `name/` contributes a mapping at key `name`.

A key may be defined in only one place. If `_index.yaml` and a sibling file, `x.yaml` and `x/`, or an inline mapping and a child file both define a key, loading fails with an error that names both sources. Apart from `_labels.yaml` below, nothing is deep-merged, and the last file does not win.

### `_labels.yaml`

The root may also hold `_labels.yaml`, which [Docker label discovery](../../docs/discovery.md#vespers-own-settings-from-labels) writes from the Vesper container's labels. It is a root-shaped mapping, such as `network: {allowed_cidrs: [10.0.0.0/8]}`, and may set anything except services (`machines.<m>.services`). It is the lowest layer: the rest of the tree is deep-merged over it. Mappings merge key by key, and any other value set elsewhere, including a list, replaces the one in `_labels.yaml`. `sources` names `_labels.yaml` for a section or machine that only it defines. Saves never edit it; a changed value is written to the files above it, where it takes precedence.

Splitting is allowed only at these points:

| Directory | `_index.yaml` holds | Files and directories allowed |
| --- | --- | --- |
| root | any top-level field | `defaults.yaml`, `network.yaml`, `telegram.yaml`, `notifications.yaml`, `machines.yaml` or `machines/` |
| `machines/` | machines inline by ID | `<machine>.yaml` or `<machine>/` |
| `machines/<m>/` | machine fields | `services.yaml` or `services/` |
| `services/` | services inline by ID | `<service>.yaml` or `<service>/` |
| `services/<s>/` | service fields | `checks.yaml` or `checks/` |
| `checks/` | checks inline by ID | `<check>.yaml` (checks are never directories) |

These layouts are equivalent:

```text
_index.yaml                      # everything inline
```

```text
_index.yaml                      # defaults
network.yaml
machines/jelly/_index.yaml       # name, site, host_service
machines/jelly/services/host.yaml
machines/jelly/services/jellyfin/_index.yaml
machines/jelly/services/jellyfin/checks/web.yaml
machines/jelly/services/jellyfin/checks/_index.yaml   # more checks inline
```

IDs (file and directory names in collections, and inline keys) are 1 to 80 ASCII letters, digits, `-` and `_`. `_index` is reserved. Within a collection, IDs must not differ only by case.

Hidden entries (names that start with `.`, such as `.git` or editor swap files) are ignored. Any other non-`.yaml` file is an error, and `.yml` is rejected with a hint. An empty file is an empty mapping.

## YAML subset

Supported: block and flow collections, plain, quoted and block scalars, and comments. JSON is accepted, since it is valid flow YAML.

Rejected: duplicate keys, anchors and aliases, tags (`!env`), merge keys (`<<`), complex keys, and multiple documents. Plain scalars resolve with the YAML 1.2 core schema, so `yes` and `on` are strings and must not be used for booleans. Quote numbers that are meant to be strings, such as `name: "1984"`.

Secret references are mappings: `{env: NAME}` or `{file: name}`. They are written the same way in YAML and in API JSON (`{"env": "NAME"}`).

## Bounds and confinement

- Each file can be at most 1 MiB, the whole tree at most 16 MiB, and the tree can hold at most 10,000 entries.
- Directories nest at most 6 deep, and collections nest at most 32 deep inside a file.
- Symlinks, devices, FIFOs and sockets anywhere below the root are errors. The root path itself may be a symlink, such as a mount point.
- Every read and write is descriptor-relative with `O_NOFOLLOW` at each component. Swapping an ancestor directory for a symlink therefore cannot redirect access outside the root.
- `secrets::resolve` accepts a file name, not a path: one segment of `[A-Za-z0-9_][A-Za-z0-9_.-]*`. The file may be a symlink, as in Kubernetes secret mounts, only if its canonical target stays inside the secrets directory.
- Secret files are UTF-8 and at most 64 KiB. One trailing newline is removed, and empty values are rejected. `network.private_ca_file` follows the same file-name rule.

## Validation

`config::load` validates before it returns, and returns nothing partial on any error. Callers keep their last valid `LoadedConfig`. Error messages name files and logical paths. Quoted values from serde errors are redacted, because a literal token pasted where a reference belongs must not be echoed back.

Validation covers:
- Policies: interval 1 s to 7 d, timeout 1 to 300 s and no longer than the interval, thresholds 1 to 100, and stale after at least the interval. Each check is checked again after its overrides are applied.
- Check kinds:
  - http: an http(s) URL with no userinfo, status 100 to 599, and header values that are secret references. Host, Connection, Content-Length, Transfer-Encoding, Upgrade and TE cannot be set.
  - tcp: a host and a port.
  - push: a token reference.
  - Fields that belong to another kind are rejected.
- `depends_on` must name an existing `machine/service`. A service cannot depend on itself, list the same dependency twice, or form a cycle. `host_service` must be on the same machine.
- Maintenance windows use RFC3339 timestamps with the end after the start. A weekly window can be at most 7 days.
- Network CIDRs and limits, the dashboard URL, the icon ID format, and notification channels: a known `kind`, only that kind's settings, required settings present, secret references for credentials ([notifications](../../docs/notifications.md)).

## Saving

`save_service` and `save_machine` take the revision the caller loaded. A different revision returns `SaveError::Conflict`. A non-writable root returns `SaveError::ReadOnly`.

A save edits only the files that define what changed:
- A changed scalar is replaced in place. Trailing comments on its line are kept.
- A removed key loses its lines, or its text and separator in a flow mapping or JSON.
- A new key is appended to its mapping. Flow mappings stay on one line, and JSON stays JSON.
- A new service in a directory-backed collection gets its own file. It gets a directory if its siblings use directories.
- A removed split check deletes its file.
- Fields left at their defaults are not written.

Before anything is written, the edited tree is assembled in memory and must equal the requested configuration. The following edits are refused with `SaveError::ExportOnly`, and nothing is written; the caller should offer `export_service`:
- A changed value would drop a comment inside it. For example, replacing a list that has a comment on an item other than the last.
- A layout the patcher does not recognize.
- Removing a directory-backed entry.
- A verification mismatch.

`save_machine` on an existing machine changes only machine fields. `machine.services` must be empty or identical to the current services. A new machine may include its services.

## Whole-file edits

`edit_files` lets a tool that owns entire files, such as [Docker label discovery](../../docs/discovery.md), write or delete `.yaml` files under the same lock as the save functions. Its planner sees the current configuration and file texts and states the configuration its changes must produce; nothing is written unless the edited tree meets the loader's size, entry-count and directory-depth limits and assembles and validates to exactly that. Missing parent directories are created, and the same conflict detection applies. A dry run verifies the same limits without writing. UI saves enforce these limits too.

## Known limitations

- **Concurrent external edits.** Vesper's own readers and writers serialize on an `flock` of the root. A save re-reads the whole tree after staging its files and before renaming them into place, and aborts with `Conflict` if any file changed, including files it does not touch. A non-cooperating editor that writes between that re-read and the renames, a window of microseconds, can still be overwritten.
- **Multi-file saves are not transactional.** Each file is replaced atomically by rename, with the file and the directory fsynced. A crash in the middle of an edit that spans several files can leave some of them updated. The server keeps the last valid configuration if the result does not validate.
- **Formatting of inserted text.** New keys use two-space block style, or inline flow inside flow mappings. Inserted entries in pretty-printed multi-line JSON are appended on the last entry's line. Comments directly above a removed entry stay where they are.
- Edits always write `\n` line endings, even into CRLF files.
- Kept block scalars (`|+`) that end a mapping can make an append unverifiable, which returns `ExportOnly`.
- A save of an unchanged service writes nothing.
