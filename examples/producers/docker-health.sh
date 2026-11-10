#!/usr/bin/env bash
# Report Docker daemon and container health to a Vesper push check.
#
# Runs ON the Docker host (systemd timer, see examples/systemd/), reads local
# container state and pushes the result outbound. Vesper never receives the
# Docker socket and never executes anything on this host.
#
# Environment:
#   VESPER_CHECK_ID           e.g. docker1/host/containers
#   VESPER_DOCKER_CONTAINERS  space-separated container names that must be
#                             running (and healthy, if they define a
#                             HEALTHCHECK), e.g. "traefik vesper jellyfin"
#   VESPER_URL, VESPER_TOKEN_FILE, VESPER_CA_FILE  as for vesper-push.sh
#
# Healthy means: `docker info` answers, and every listed container exists, is
# running, is not restarting, and is not unhealthy/starting. Anything else is
# reported as unhealthy with the failing container names printed locally
# (names and states only; no inspect output, labels or environment).
#
# Reading Docker state requires access to the local Docker socket, which is
# root-equivalent on this host. Keep this script root-owned and unwritable by
# others, and run it under the hardened unit in examples/systemd/.
set -euo pipefail

here=$(dirname "$(readlink -f "$0")")
push=${VESPER_PUSH:-$here/vesper-push.sh}
check_id=${VESPER_CHECK_ID:?set VESPER_CHECK_ID, e.g. docker1/host/containers}
read -r -a containers <<<"${VESPER_DOCKER_CONTAINERS:-}"

problems=()
if ! timeout 10 docker info --format '{{.ServerVersion}}' >/dev/null 2>&1; then
  problems+=("docker daemon not responding")
else
  for name in "${containers[@]}"; do
    if ! state=$(timeout 10 docker inspect --type container \
      --format '{{.State.Status}} {{.State.Restarting}} {{if .State.Health}}{{.State.Health.Status}}{{else}}none{{end}}' \
      -- "$name" 2>/dev/null); then
      problems+=("$name: missing")
      continue
    fi
    read -r status restarting health <<<"$state"
    if [[ $status != running || $restarting == true ]]; then
      problems+=("$name: $status")
    elif [[ $health != healthy && $health != none ]]; then
      problems+=("$name: $health")
    fi
  done
fi

if ((${#problems[@]} == 0)); then
  echo "docker healthy: ${#containers[@]} required containers running"
  exec "$push" "$check_id" true
else
  (IFS=';'; echo "docker unhealthy: ${problems[*]}")
  exec "$push" "$check_id" false
fi
