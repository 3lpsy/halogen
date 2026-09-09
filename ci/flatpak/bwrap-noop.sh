#!/bin/sh
# Flatpak icon validation shim: drop sandbox flags and exec after -- because bwrap cannot nest here.
# Validates our own icon unsandboxed. An empty environment requires /bin/sh and absolute paths.
while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do shift; done
if [ "$#" -eq 0 ]; then
  echo "bwrap-noop: no '--' separator in argv" >&2
  exit 1
fi
shift
exec "$@"
