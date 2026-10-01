#!/bin/sh
# M6-T7 (rider: exit-printing by construction) — run a gate/instrument
# command and ALWAYS leave the two charter faces in the log: the exact
# invocation echo as the FIRST line and `exit=N` as the LAST line.
#
# usage: run-gate.sh <label> <cmd> [args...]
#   <label> — a short face name echoed between the invocation and the
#             command's own output.
# The wrapper exits with the command's own exit code (never read
# through a pipe: redirect the WHOLE wrapper's output to the log).
set -u
if [ "$#" -lt 2 ]; then
  echo "usage: run-gate.sh <label> <cmd> [args...]" >&2
  exit 2
fi
label="$1"
shift
printf '# Invocation:'
# shellcheck disable=SC3050  # %q is a deliberate bashism here (the invocation echo)
printf ' %q' "$@"
printf '\n'
printf '# label: %s\n' "$label"
"$@"
status=$?
printf 'exit=%s\n' "$status"
exit "$status"
