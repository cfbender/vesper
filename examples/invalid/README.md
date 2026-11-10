# Configurations Vesper must reject

Each subdirectory is a complete configuration root that `vesper validate` must refuse with a nonzero exit status and a safe error on stderr. The CI container smoke test ([`deploy/smoke-test.sh`](../../deploy/smoke-test.sh)) runs every case. Use them to check that a build enforces the layout rules before trusting it with real inventory.

| Case | Rule it breaks |
| --- | --- |
| `file-directory-collision` | `machines/nas.yaml` and `machines/nas/` both define machine `nas`. |
| `index-child-collision` | `machines/docker1/_index.yaml` defines `services` inline while `machines/docker1/services/` also exists. |
| `duplicate-key` | One YAML mapping repeats the key `port`. |
| `literal-header-secret` | A check header holds a literal credential instead of `{env: ...}` or `{file: ...}`. |
| `unknown-field` | A check uses `interval`, which is not a field. |
| `invalid-id` | Service ID `dns.resolver` contains a character outside `A-Z a-z 0-9 - _`. |
