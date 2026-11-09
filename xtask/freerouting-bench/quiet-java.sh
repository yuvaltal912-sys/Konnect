#!/bin/sh
# A `java` for tools that start Freerouting themselves (such as PR 16's ignored
# test `freerouting_routes_a_folder`): each start gets a throwaway home and temp
# folder, and the GUI, telemetry and analytics are off. Freerouting 2.x turns
# telemetry on by default and keeps its settings and log in the user's home.
# HTTP and HTTPS go to a closed loopback port, so 2.5.0's version check (a GET
# to api.github.com on every start, with no switch) never leaves the machine.
#
#   FREEROUTING_JAVA=xtask/freerouting-bench/quiet-java.sh
#
# QUIET_JAVA names the real Java (default: Homebrew's openjdk@25).
real=${QUIET_JAVA:-/opt/homebrew/opt/openjdk@25/bin/java}
home=$(mktemp -d "${TMPDIR:-/tmp}/freerouting-home.XXXXXX") || exit 1
exec "$real" -Djava.awt.headless=true -Duser.home="$home" -Djava.io.tmpdir="$home" \
  -Dhttp.proxyHost=127.0.0.1 -Dhttp.proxyPort=9 \
  -Dhttps.proxyHost=127.0.0.1 -Dhttps.proxyPort=9 \
  '-Dhttp.nonProxyHosts=localhost|127.*|[::1]' "$@" \
  --gui.enabled=false --profile.allow_telemetry=false \
  --usage_and_diagnostic_data.disable_analytics=true
