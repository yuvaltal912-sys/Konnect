#!/bin/sh
# A `java` for tools that start Freerouting themselves (such as PR 16's ignored
# test `freerouting_routes_a_folder`): each start gets a throwaway home and temp
# folder, and the GUI, telemetry and analytics are off. Freerouting 2.x turns
# telemetry on by default and keeps its settings and log in the user's home.
#
#   FREEROUTING_JAVA=xtask/freerouting-bench/quiet-java.sh
#
# QUIET_JAVA names the real Java (default: Homebrew's openjdk@25).
real=${QUIET_JAVA:-/opt/homebrew/opt/openjdk@25/bin/java}
home=$(mktemp -d "${TMPDIR:-/tmp}/freerouting-home.XXXXXX") || exit 1
exec "$real" -Djava.awt.headless=true -Duser.home="$home" -Djava.io.tmpdir="$home" "$@" \
  --gui.enabled=false --profile.allow_telemetry=false \
  --usage_and_diagnostic_data.disable_analytics=true
