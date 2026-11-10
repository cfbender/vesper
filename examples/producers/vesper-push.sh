#!/usr/bin/env bash
# Send one push heartbeat to Vesper.
#
#   vesper-push.sh MACHINE/SERVICE/CHECK true|false
#
# Environment:
#   VESPER_URL         Vesper's public origin, e.g. https://status.example.com
#   VESPER_TOKEN_FILE  File holding this check's bearer token. Defaults to
#                      $CREDENTIALS_DIRECTORY/vesper-token, which systemd
#                      provides through LoadCredential=vesper-token:<path>.
#   VESPER_CA_FILE     Optional PEM bundle for a private CA on the proxy.
#
# The token travels only in the Authorization header, fed to curl on stdin so
# it never appears in a process argument list, URL, shell trace or log. The
# script prints the HTTP status, never the token or request headers. It exits
# nonzero when Vesper did not accept the heartbeat; a heartbeat that never
# arrives is itself detected by Vesper as a missed deadline.
set -euo pipefail
set +x

usage() {
  echo "usage: $0 MACHINE/SERVICE/CHECK true|false" >&2
  exit 64
}

[[ $# -eq 2 ]] || usage
check_id=$1
healthy=$2
id_re='^[A-Za-z0-9_-]+/[A-Za-z0-9_-]+/[A-Za-z0-9_-]+$'
[[ $check_id =~ $id_re ]] || { echo "invalid check ID: $check_id" >&2; exit 64; }
[[ $healthy == true || $healthy == false ]] || usage

url=${VESPER_URL:?set VESPER_URL to Vesper\'s https origin}
url=${url%/}
[[ $url == https://* ]] || { echo "VESPER_URL must use https" >&2; exit 64; }

token_file=${VESPER_TOKEN_FILE:-${CREDENTIALS_DIRECTORY:+$CREDENTIALS_DIRECTORY/vesper-token}}
[[ -n $token_file && -r $token_file ]] || { echo "token file is not readable" >&2; exit 66; }
IFS= read -r token <"$token_file" || true
token=${token%$'\r'}
[[ -n $token ]] || { echo "token file is empty" >&2; exit 66; }

curl_args=(
  --silent --show-error
  --proto "=https" --tlsv1.2
  --max-time 15 --connect-timeout 5
  --retry 2 --retry-delay 2 --retry-connrefused
  --output /dev/null --write-out '%{http_code}'
  --header @-
  --header 'Content-Type: application/json'
  --data-binary "{\"healthy\":$healthy}"
)
[[ -n ${VESPER_CA_FILE:-} ]] && curl_args+=(--cacert "$VESPER_CA_FILE")

# printf is a shell builtin, so the token is not visible in `ps`.
status=$(printf 'Authorization: Bearer %s\n' "$token" |
  curl "${curl_args[@]}" "$url/api/heartbeat/$check_id") || status=${status:-000}
unset token

if [[ $status == 2?? ]]; then
  echo "heartbeat $check_id healthy=$healthy accepted (HTTP $status)"
else
  echo "heartbeat $check_id healthy=$healthy NOT accepted (HTTP $status)" >&2
  exit 75
fi
