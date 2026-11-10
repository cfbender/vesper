#!/usr/bin/env bash
# Report Proxmox VE node, quorum and guest health to a Vesper push check.
#
# Runs on a Proxmox node as an unprivileged user (systemd timer, see
# examples/systemd/) and reads the Proxmox API with a privilege-separated API
# token that holds only the read-only PVEAuditor role. It never uses root,
# SSH, pvesh/qm as root, or remote command execution, and Vesper never
# connects to Proxmox for this check: the result is pushed outbound.
#
# Environment:
#   VESPER_CHECK_ID  e.g. pve1/host/cluster-health
#   PVE_API_URL      default https://localhost:8006
#   PVE_TOKEN_FILE   file containing USER@REALM!TOKENID=SECRET; defaults to
#                    $CREDENTIALS_DIRECTORY/pve-token (systemd LoadCredential)
#   PVE_CA_FILE      CA for the API certificate; defaults to
#                    $CREDENTIALS_DIRECTORY/pve-ca when present, else the
#                    system trust store. TLS verification is never disabled.
#   PVE_NODES        nodes that must be online (default: this host's name)
#   PVE_GUESTS       VMIDs (QEMU or LXC) that must be running, e.g. "101 102"
#   VESPER_URL, VESPER_TOKEN_FILE, VESPER_CA_FILE  as for vesper-push.sh
#
# Healthy means: the API answers, the cluster is quorate (when clustered),
# each listed node is online and each listed guest is running. Anything else,
# including an API or authentication error, is reported as unhealthy.
set -euo pipefail
set +x

here=$(dirname "$(readlink -f "$0")")
push=${VESPER_PUSH:-$here/vesper-push.sh}
check_id=${VESPER_CHECK_ID:?set VESPER_CHECK_ID, e.g. pve1/host/cluster-health}
api=${PVE_API_URL:-https://localhost:8006}
api=${api%/}
token_file=${PVE_TOKEN_FILE:-${CREDENTIALS_DIRECTORY:+$CREDENTIALS_DIRECTORY/pve-token}}
ca_file=${PVE_CA_FILE:-}
if [[ -z $ca_file && -n ${CREDENTIALS_DIRECTORY:-} && -r $CREDENTIALS_DIRECTORY/pve-ca ]]; then
  ca_file=$CREDENTIALS_DIRECTORY/pve-ca
fi
read -r -a nodes <<<"${PVE_NODES:-$(hostname -s)}"
read -r -a guests <<<"${PVE_GUESTS:-}"
for vmid in "${guests[@]}"; do
  [[ $vmid =~ ^[0-9]+$ ]] || { echo "PVE_GUESTS must list numeric VMIDs" >&2; exit 64; }
done

[[ -n $token_file && -r $token_file ]] || { echo "PVE token file is not readable" >&2; exit 66; }
IFS= read -r pve_token <"$token_file" || true
pve_token=${pve_token%$'\r'}

pve_get() {
  local args=(--silent --show-error --fail --proto "=https" --max-time 10 --header @-)
  [[ -n $ca_file ]] && args+=(--cacert "$ca_file")
  printf 'Authorization: PVEAPIToken=%s\n' "$pve_token" | curl "${args[@]}" "$api/api2/json/$1"
}

problems=()
if ! resources=$(pve_get cluster/resources 2>/dev/null) || ! status=$(pve_get cluster/status 2>/dev/null); then
  problems+=("Proxmox API request failed (check URL, CA and token)")
else
  quorate=$(jq -r '[.data[] | select(.type == "cluster") | .quorate] | first // "standalone"' <<<"$status")
  if [[ $quorate != 1 && $quorate != standalone ]]; then
    problems+=("cluster not quorate")
  fi
  for node in "${nodes[@]}"; do
    state=$(jq -r --arg n "$node" '[.data[] | select(.type == "node" and .node == $n) | .status] | first // "missing"' <<<"$resources")
    [[ $state == online ]] || problems+=("node $node: $state")
  done
  for vmid in "${guests[@]}"; do
    state=$(jq -r --argjson id "$vmid" '[.data[] | select((.type == "qemu" or .type == "lxc") and .vmid == $id) | .status] | first // "missing"' <<<"$resources")
    [[ $state == running ]] || problems+=("guest $vmid: $state")
  done
fi
unset pve_token

if ((${#problems[@]} == 0)); then
  echo "proxmox healthy: ${#nodes[@]} nodes online, ${#guests[@]} guests running"
  exec "$push" "$check_id" true
else
  (IFS=';'; echo "proxmox unhealthy: ${problems[*]}")
  exec "$push" "$check_id" false
fi
