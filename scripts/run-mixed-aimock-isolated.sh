#!/bin/sh
set -eu
# Run on the dedicated Linux test host after dependencies are installed.
# A private network namespace has no route to external hosts.
if [ "$(uname -s)" != "Linux" ]; then
  echo "The isolated periodic runner requires Linux." >&2
  exit 2
fi
exec unshare --user --map-root-user --net sh -eu -c '
  ip link set lo up
  export MALLARD_MIXED_NETWORK_ISOLATED=1
  export MALLARD_RUN_MIXED_AIMOCK_E2E=1
  exec node scripts/run-mixed-aimock-e2e.mjs "$@"
' mixed-aimock "$@"
