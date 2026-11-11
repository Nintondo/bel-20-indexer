#!/usr/bin/env sh
# The runner-managed configuration exists only for this BuildKit RUN.
set -eu
if [ ! -r /run/secrets/cargo_cache_env ]; then
  exec "$@"
fi
set -a
# shellcheck disable=SC1091
. /run/secrets/cargo_cache_env
set +a
export RUSTC_WRAPPER=/usr/local/bin/sccache CARGO_INCREMENTAL=0
export SCCACHE_IGNORE_SERVER_IO_ERROR=1
# Isolate statistics and cleanup from other compilation instructions/jobs.
cache_state="$(mktemp -d)"
export SCCACHE_SERVER_UDS="$cache_state/server.sock"
# Invoked by the EXIT trap.
# shellcheck disable=SC2329
cleanup() {
  /usr/local/bin/sccache --stop-server >/dev/null 2>&1 || :
  rm -rf "$cache_state"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
if "$@"; then
  result=0
else
  result=$?
fi
# Exclude cache location/configuration: these can contain endpoint credentials.
/usr/local/bin/sccache --show-stats 2>/dev/null | sed -n \
  -e '/^Compile requests[[:space:]]/p' \
  -e '/^Cache hits[[:space:]]/p' \
  -e '/^Cache misses[[:space:]]/p' \
  -e '/^Cache read errors[[:space:]]/p' \
  -e '/^Cache write errors[[:space:]]/p' \
  -e '/^Non-cacheable calls[[:space:]]/p' \
  -e '/^Average cache [[:alnum:] ]*duration[[:space:]]/p' || :
exit "$result"
