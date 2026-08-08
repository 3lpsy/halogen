#!/bin/sh
# bwrap stand-in for flatpak's icon validation ($FLATPAK_BWRAP): drops the
# sandbox flags and execs everything after "--". Real bwrap can't nest here;
# the validation still runs, just unsandboxed — the input is our own icon.
# Invoked with an EMPTY environment: keep /bin/sh + absolute paths only.
while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do shift; done
if [ "$#" -eq 0 ]; then
  echo "bwrap-noop: no '--' separator in argv" >&2
  exit 1
fi
shift
exec "$@"
