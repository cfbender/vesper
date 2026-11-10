# Examples

| Path | What it is |
| --- | --- |
| [`config/`](config) | A complete, valid configuration root: 4 machines, 10 services, 13 HTTP, TCP and push checks. It uses every layout form: a single-file machine, a directory machine, per-service files, and a service split into per-check files. |
| [`invalid/`](invalid) | Configuration roots that `vesper validate` must reject: collisions, duplicate keys, literal secrets, unknown fields and invalid IDs. |
| [`producers/`](producers) | Outbound push producers. `vesper-push.sh` sends one heartbeat. `docker-health.sh` and `proxmox-health.sh` report real host health. `run-and-report.sh` wraps a scheduled job. |
| [`systemd/`](systemd) | Hardened service and timer units and env files for the producers and the watchdog. |
| [`watchdog/`](watchdog) | The independent watchdog, which proves that Vesper accepts heartbeats and reports to Uptime Kuma. |

Secret references in `config/` name files under `VESPER_SECRETS_DIR`. None of them are included. Start with the small configuration in the [installation guide](../docs/installation.md), or adapt this inventory to your targets and provision its referenced secrets. Producer scripts document their required variables; adjust the systemd units' paths, credentials and schedules to your hosts.

Every producer:

- sends its token only as `Authorization: Bearer …`, on curl's stdin, never in a URL, argument or log;
- refuses plain HTTP and keeps TLS verification on (`VESPER_CA_FILE` adds a private CA);
- reports `healthy:false` when the host answers but is unhealthy, and exits nonzero when Vesper did not accept the heartbeat;
- relies on Vesper's deadline (`expected_interval_seconds + grace_seconds`) to catch the case where no heartbeat arrives at all.
