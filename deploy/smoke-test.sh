#!/usr/bin/env bash
# pass/fail always succeed, so `cond && pass || fail` is safe in this file.
# shellcheck disable=SC2015
# Container smoke test for a built Vesper image.
#
#   docker build -t vesper:smoke . && deploy/smoke-test.sh vesper:smoke
#
# Runs the image exactly as deploy/compose.yaml does (UID 10001, read-only
# root, no capabilities, no-new-privileges, no published ports) and checks:
#   - `vesper validate` accepts examples/config and rejects examples/invalid/*
#     without echoing secret material
#   - the server becomes healthy through `vesper healthcheck`
#   - a trusted peer gets the populated inventory; an untrusted peer gets 403
#   - heartbeats need the per-check bearer token and leak no metadata
#   - mutations need X-Vesper-CSRF and the exact Origin
#   - read-only config is reported read-only; state survives a restart
#   - no secret value appears in API responses or container logs
#
# Needs docker and jq. Pulls CURL_IMAGE for the in-network clients. Leaves
# nothing behind: containers, networks, volumes and temp files are removed.
set -euo pipefail

image=${1:-vesper:smoke}
repo=$(cd "$(dirname "$0")/.." && pwd)
curl_image=${CURL_IMAGE:-curlimages/curl:8.11.1}
origin=https://status.example.test
run_id=vesper-smoke-$$
trusted_net=$run_id-trusted
untrusted_net=$run_id-untrusted
trusted_cidr=${SMOKE_TRUSTED_CIDR:-172.31.251.0/28}
untrusted_cidr=${SMOKE_UNTRUSTED_CIDR:-172.31.252.0/28}
tmp=$(mktemp -d)
failures=0

cleanup() {
  docker rm -f "$run_id" "$run_id-ct" "$run_id-cu" >/dev/null 2>&1 || true
  docker network rm "$trusted_net" "$untrusted_net" >/dev/null 2>&1 || true
  docker volume rm "$run_id-data" >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT

pass() { printf 'PASS  %s\n' "$*"; }
fail() { printf 'FAIL  %s\n' "$*"; failures=$((failures + 1)); }
jqr() { jq -r "$@" 2>/dev/null || echo "<invalid JSON>"; }
expect() { # expect DESCRIPTION EXPECTED ACTUAL
  if [[ $3 == "$2" ]]; then pass "$1 ($3)"; else fail "$1: expected $2, got $3"; fi
}

hardening=(--user 10001:10001 --read-only --cap-drop ALL --security-opt no-new-privileges:true
  --tmpfs "/tmp:size=16m,mode=1777,noexec,nosuid,nodev")

# --- Dummy secrets for every reference in examples/config -----------------
secrets=$tmp/secrets
mkdir -m 0755 "$secrets"
for name in telegram-bot-token paperless-api-authorization push-pve1-host-cluster-health \
  push-docker1-host-containers push-docker1-vesper-watchdog push-nas-backups-restic; do
  printf 'smoke-%s-%s\n' "$name" "$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')" >"$secrets/$name"
done
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj /CN=vesper-smoke-ca \
  -keyout "$tmp/ca.key" -out "$secrets/homelab-ca.pem" >/dev/null 2>&1
chmod 0444 "$secrets"/*
chat_id=-100$(od -An -N4 -tu4 /dev/urandom | tr -d ' ')
secret_values=("$chat_id")
for f in "$secrets"/*; do [[ $f == *.pem ]] || secret_values+=("$(<"$f")"); done
push_token=$(<"$secrets/push-docker1-host-containers")

leaks() { # leaks TEXT: print names of secret values that occur in TEXT
  local v
  for v in "${secret_values[@]}"; do [[ $1 == *"$v"* ]] && echo "${v:0:24}..."; done
  [[ $1 == *do-not-put-credentials-in-yaml* ]] && echo "literal header credential"
  return 0
}

# --- Image metadata -------------------------------------------------------
expect "image user" 10001:10001 "$(docker image inspect -f '{{.Config.User}}' "$image")"
expect "image healthcheck" "[CMD /usr/local/bin/vesper healthcheck]" \
  "$(docker image inspect -f '{{.Config.Healthcheck.Test}}' "$image")"

# --- validate -------------------------------------------------------------
validate() { # validate CONFIG_DIR -> prints exit code; stderr in $tmp/validate.err
  local rc=0
  docker run --rm "${hardening[@]}" --network none \
    -e VESPER_TELEGRAM_CHAT_ID="$chat_id" \
    -v "$1:/config:ro" -v "$secrets:/run/secrets:ro" \
    "$image" validate >"$tmp/validate.out" 2>"$tmp/validate.err" || rc=$?
  echo "$rc"
}
rc=$(validate "$repo/examples/config")
if [[ $rc == 0 ]]; then pass "validate examples/config"; else
  fail "validate examples/config exited $rc: $(head -c 500 "$tmp/validate.err")"
fi
for case in "$repo"/examples/invalid/*/; do
  name=$(basename "$case")
  rc=$(validate "$case")
  err=$(cat "$tmp/validate.out" "$tmp/validate.err")
  if [[ $rc != 0 && -s $tmp/validate.err ]]; then pass "validate rejects $name"; else
    fail "validate $name: exit $rc, stderr $(wc -c <"$tmp/validate.err") bytes"
  fi
  leaked=$(leaks "$err")
  [[ -z $leaked ]] || fail "validate $name output leaks: $leaked"
done

# --- serve ----------------------------------------------------------------
docker pull -q "$curl_image" >/dev/null
docker network create --subnet "$trusted_cidr" "$trusted_net" >/dev/null
docker network create --subnet "$untrusted_cidr" "$untrusted_net" >/dev/null
docker volume create "$run_id-data" >/dev/null
docker run -d --name "$run_id" "${hardening[@]}" --network "$trusted_net" \
  -e VESPER_PUBLIC_URL="$origin" \
  -e VESPER_TRUSTED_PROXIES="$trusted_cidr" \
  -e VESPER_CONFIG_READ_ONLY=true \
  -e VESPER_TELEGRAM_CHAT_ID="$chat_id" \
  -v "$repo/examples/config:/config:ro" \
  -v "$run_id-data:/data" \
  -v "$secrets:/run/secrets:ro" \
  "$image" >/dev/null
docker network connect "$untrusted_net" "$run_id"
for net in ct:"$trusted_net" cu:"$untrusted_net"; do
  docker run -d --name "$run_id-${net%%:*}" --network "${net#*:}" --entrypoint sleep "$curl_image" 600 >/dev/null
done

wait_healthy() {
  local state
  for _ in $(seq 60); do
    state=$(docker inspect -f '{{.State.Health.Status}}' "$run_id")
    [[ $state == healthy ]] && { pass "container healthy"; return 0; }
    [[ $(docker inspect -f '{{.State.Running}}' "$run_id") == true ]] || break
    sleep 1
  done
  fail "container not healthy (state: ${state:-unknown})"
  docker logs --tail 40 "$run_id" 2>&1 | sed 's/^/      log: /'
  exit 1
}
wait_healthy

# req CLIENT METHOD PATH [curl args...] -> body in $tmp/body, prints status
req() {
  local client=$1 method=$2 path=$3
  shift 3
  docker exec "$run_id-$client" curl -s -o /dev/stdout -w '\n%{http_code}' -X "$method" \
    "$@" "http://$run_id:8080$path" >"$tmp/raw" || true
  tail -n1 "$tmp/raw"
  sed '$d' "$tmp/raw" >"$tmp/body"
}

expect "trusted GET /healthz" 200 "$(req ct GET /healthz)"
expect "untrusted GET /healthz (public liveness)" 200 "$(req cu GET /healthz)"
expect "trusted GET /" 200 "$(req ct GET /)"
grep -qi '<html' "$tmp/body" && pass "/ serves the web UI" || fail "/ did not return HTML"
expect "untrusted GET /api/snapshot" 403 "$(req cu GET /api/snapshot)"
jq -e '.error | strings' "$tmp/body" >/dev/null 2>&1 && pass "403 body is {error}" || fail "403 body is not {error}"
expect "untrusted GET /" 403 "$(req cu GET /)"

expect "trusted GET /api/snapshot" 200 "$(req ct GET /api/snapshot)"
snapshot=$(cat "$tmp/body")
counts=$(jqr '[(.machines | length), ([.machines[].services[]] | length), ([.machines[].services[].checks[]] | length)] | join("/")' <<<"$snapshot")
expect "snapshot machines/services/checks" 4/10/13 "$counts"
expect "snapshot config_writable" false "$(jqr '.config_writable' <<<"$snapshot")"
expect "snapshot config_error" null "$(jqr '.config_error' <<<"$snapshot")"

expect "trusted GET /api/config" 200 "$(req ct GET /api/config)"
config=$(cat "$tmp/body")
expect "/api/config writable" false "$(jqr '.writable' <<<"$config")"
leaked=$(leaks "$snapshot$config")
[[ -z $leaked ]] && pass "API responses contain no secret values" || fail "API responses leak: $leaked"

# Heartbeats (the trusted client stands in for Traefik's heartbeat route).
hb=(-H 'Content-Type: application/json' --data '{"healthy":true}')
expect "heartbeat without token" 401 "$(req ct POST /api/heartbeat/docker1/host/containers "${hb[@]}")"
expect "heartbeat wrong token" 401 "$(req ct POST /api/heartbeat/docker1/host/containers "${hb[@]}" -H 'Authorization: Bearer wrong')"
wrong_body=$(cat "$tmp/body")
expect "heartbeat unknown check" 401 "$(req ct POST /api/heartbeat/nope/nope/nope "${hb[@]}" -H 'Authorization: Bearer wrong')"
[[ $(cat "$tmp/body") == "$wrong_body" ]] && pass "unknown check and wrong token are indistinguishable" ||
  fail "unknown-check response differs from wrong-token response"
expect "heartbeat token for another check" 401 \
  "$(req ct POST /api/heartbeat/docker1/vesper/watchdog "${hb[@]}" -H "Authorization: Bearer $push_token")"
status=$(req ct POST /api/heartbeat/docker1/host/containers "${hb[@]}" -H "Authorization: Bearer $push_token")
[[ $status == 2?? ]] && pass "heartbeat with correct token ($status)" || fail "heartbeat with correct token: $status"

check_health() { # check_health -> health of docker1/host/containers in a fresh snapshot
  req ct GET /api/snapshot >/dev/null
  jqr '.machines[] | select(.id == "docker1") | .services[] | select(.id == "docker1/host" or .id == "host")
    | .checks[] | select(.id == "docker1/host/containers" or .id == "containers") | .health' "$tmp/body"
}
expect "pushed check health" healthy "$(check_health)"

# CSRF and Origin on a harmless mutation.
expect "reload without CSRF" 403 "$(req ct POST /api/config/reload -H "Origin: $origin")"
req ct GET /api/csrf >/dev/null
csrf=$(jqr '.token' "$tmp/body")
[[ -n $csrf && $csrf != null && $csrf != "<invalid JSON>" ]] && pass "GET /api/csrf returns a token" || fail "no CSRF token"
expect "reload with CSRF, wrong Origin" 403 \
  "$(req ct POST /api/config/reload -H "X-Vesper-CSRF: $csrf" -H 'Origin: https://evil.example')"
expect "reload with CSRF, no Origin" 403 "$(req ct POST /api/config/reload -H "X-Vesper-CSRF: $csrf")"
expect "untrusted reload with CSRF" 403 \
  "$(req cu POST /api/config/reload -H "X-Vesper-CSRF: $csrf" -H "Origin: $origin")"
status=$(req ct POST /api/config/reload -H "X-Vesper-CSRF: $csrf" -H "Origin: $origin")
[[ $status == 2?? ]] && pass "reload with CSRF and exact Origin ($status)" || fail "reload with CSRF and Origin: $status"

# Container hardening as actually applied.
expect "PID 1 UID" 10001 "$(docker exec "$run_id" awk '/^Uid:/ {print $2}' /proc/1/status)"
expect "effective capabilities" 0000000000000000 "$(docker exec "$run_id" awk '/^CapEff:/ {print $2}' /proc/1/status)"
expect "no_new_privs" 1 "$(docker exec "$run_id" awk '/^NoNewPrivs:/ {print $2}' /proc/1/status)"
docker exec "$run_id" sh -c ': >/probe' 2>/dev/null && fail "root filesystem is writable" || pass "root filesystem is read-only"
docker exec "$run_id" sh -c ': >/config/probe' 2>/dev/null && fail "/config is writable" || pass "/config is read-only"
docker exec "$run_id" test -s /data/vesper.db && pass "/data/vesper.db exists" || fail "/data/vesper.db missing"
expect "published ports" "map[]" "$(docker inspect -f '{{.HostConfig.PortBindings}}' "$run_id")"

# Restart keeps state; stop is prompt.
start=$(date +%s)
docker stop "$run_id" >/dev/null
elapsed=$(($(date +%s) - start))
((elapsed < 10)) && pass "docker stop took ${elapsed}s" || fail "docker stop took ${elapsed}s"
docker start "$run_id" >/dev/null
wait_healthy
req ct GET /api/snapshot >/dev/null
history=$(jqr '[.machines[] | select(.id == "docker1") | .services[] | .checks[]
  | select(.id == "docker1/host/containers" or .id == "containers") | .history | length] | first // 0' "$tmp/body")
[[ $history =~ ^[0-9]+$ ]] || history=0
((history > 0)) && pass "heartbeat history survives restart ($history samples)" || fail "history lost after restart"

leaked=$(leaks "$(docker logs "$run_id" 2>&1)")
[[ -z $leaked ]] && pass "container logs contain no secret values" || fail "container logs leak: $leaked"

echo
if ((failures)); then
  echo "$failures check(s) failed"
  docker logs --tail 40 "$run_id" 2>&1 | sed 's/^/      log: /'
  exit 1
fi
echo "all smoke checks passed"
