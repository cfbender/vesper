#!/usr/bin/env bash
# Run a job and report its real outcome to a Vesper push check.
#
#   run-and-report.sh MACHINE/SERVICE/CHECK -- COMMAND [ARGS...]
#
# Pushes healthy=true when COMMAND exits 0 and healthy=false otherwise, then
# exits with COMMAND's status so cron/systemd still see the failure. If this
# host is down or the job never runs, no heartbeat arrives and Vesper opens an
# incident once the check's expected_interval_seconds + grace_seconds pass.
#
# Example (nightly backup on the NAS, see examples/config/machines/nas.yaml):
#   run-and-report.sh nas/backups/restic -- restic backup /mnt/tank/important
#
# VESPER_URL, VESPER_TOKEN_FILE and VESPER_CA_FILE are as for vesper-push.sh.
# The job's own output goes to the caller's stdout/stderr unchanged; keep
# credentials out of the job's command line just as this script does.
set -uo pipefail

here=$(dirname "$(readlink -f "$0")")
push=${VESPER_PUSH:-$here/vesper-push.sh}

[[ $# -ge 3 && $2 == -- ]] || { echo "usage: $0 MACHINE/SERVICE/CHECK -- COMMAND [ARGS...]" >&2; exit 64; }
check_id=$1
shift 2

"$@"
rc=$?

if ((rc == 0)); then
  "$push" "$check_id" true || echo "warning: heartbeat for $check_id was not delivered" >&2
else
  echo "job failed with exit status $rc" >&2
  "$push" "$check_id" false || echo "warning: heartbeat for $check_id was not delivered" >&2
fi
exit "$rc"
