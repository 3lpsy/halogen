#!/usr/bin/env bash
# Renders each design contact sheet to a PNG beside it, sized to its content.
# The measuring lives in render.py (chromedriver + Chrome); this stays as the
# entry point the README names. Pass paths for a subset.
exec "$(dirname "$0")/render.py" "$@"
