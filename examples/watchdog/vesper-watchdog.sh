#!/usr/bin/env bash
# Independent watchdog for Vesper.
#
# Vesper cannot alert about its own outage, so this script runs every minute
# on a DIFFERENT machine than Vesper and reports to a monitor that does not
# depend on Vesper: an Uptime Kuma "Push" monitor (the existing Kuma keeps
# running during and after the parallel comparison).
#
# Each run:
#   1. Posts a real heartbeat to Vesper's own push check (default
#      docker1/vesper/watchdog) through the public Traefik heartbeat route.
#      Acceptance proves DNS, TLS, Traefik routing, the Vesper process, token
#      validation and the SQLite write path all work.
#   2. Pings Kuma with status=up when Vesper accepted it, status=down when it
#      did not. If this host, the network or Kuma's push stops entirely, Kuma
#      alerts after its own heartbeat interval.
#
# It does not prove that Vesper's scheduler is running active checks or that
# Telegram delivery works; verify recent check results and Telegram separately.
#
# Environment:
#   VESPER_WATCHDOG_CHECK_ID  default docker1/vesper/watchdog
#   KUMA_PUSH_URL_FILE        file with the Kuma push URL without query, e.g.
#                             https://kuma.example.com/api/push/AbCdEf123;
#                             defaults to $CREDENTIALS_DIRECTORY/kuma-push-url
#   KUMA_CA_FILE              optional PEM bundle for Kuma's certificate
#   VESPER_URL, VESPER_TOKEN_FILE, VESPER_CA_FILE  as for vesper-push.sh
#
# Kuma's push API carries its token in the URL by design. The URL is read
# from a credential file and handed to curl on stdin (--config -) so it does
# not appear in process arguments or logs; treat that file as a secret.
set -uo pipefail
set +x

here=$(dirname "$(readlink -f "$0")")
push=${VESPER_PUSH:-$here/../producers/vesper-push.sh}
check_id=${VESPER_WATCHDOG_CHECK_ID:-docker1/vesper/watchdog}
kuma_file=${KUMA_PUSH_URL_FILE:-${CREDENTIALS_DIRECTORY:+$CREDENTIALS_DIRECTORY/kuma-push-url}}

[[ -n $kuma_file && -r $kuma_file ]] || { echo "Kuma push URL file is not readable" >&2; exit 66; }
IFS= read -r kuma_url <"$kuma_file" || true
kuma_url=${kuma_url%$'\r'}
[[ $kuma_url == https://* && $kuma_url != *[\"\\?]* ]] || { echo "Kuma push URL must be an https URL without a query" >&2; exit 64; }

if "$push" "$check_id" true; then
  kuma_status=up kuma_msg=vesper-accepted-heartbeat
else
  kuma_status=down kuma_msg=vesper-rejected-or-unreachable
fi

kuma_args=(--silent --show-error --fail --proto "=https" --max-time 15 --retry 2 --output /dev/null --config -)
[[ -n ${KUMA_CA_FILE:-} ]] && kuma_args+=(--cacert "$KUMA_CA_FILE")
if printf 'url = "%s?status=%s&msg=%s"\n' "$kuma_url" "$kuma_status" "$kuma_msg" | curl "${kuma_args[@]}"; then
  echo "kuma notified: status=$kuma_status"
else
  echo "kuma push failed; Kuma will alert on the missed heartbeat" >&2
  exit 75
fi
unset kuma_url
[[ $kuma_status == up ]]
