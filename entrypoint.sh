#!/bin/sh
set -e

log() { echo "[entrypoint] $*"; }
is_root() { [ "$(id -u)" = "0" ]; }

AUTO_RUN_AS_FROM_BLK_DIR="${AUTO_RUN_AS_FROM_BLK_DIR:-}"

APP_UID="${APP_UID:-}"
APP_GID="${APP_GID:-}"

if [ -n "$AUTO_RUN_AS_FROM_BLK_DIR" ] && [ -d "/app/blk-dir" ]; then
    blk_uid="$(stat -c '%u' /app/blk-dir 2>/dev/null || true)"
    blk_gid="$(stat -c '%g' /app/blk-dir 2>/dev/null || true)"
    if [ -n "$blk_uid" ] && [ -n "$blk_gid" ]; then
        APP_UID="${APP_UID:-$blk_uid}"
        APP_GID="${APP_GID:-$blk_gid}"
        log "Auto-selected UID:GID from /app/blk-dir -> $APP_UID:$APP_GID"
    fi
fi

APP_UID="${APP_UID:-1001}"
APP_GID="${APP_GID:-1001}"

FIX_PERMS_DIRS="${FIX_PERMS_DIRS:-/app /app/index-dir /app/rocksdb}"
EXCLUDE_DIRS="${EXCLUDE_DIRS:-/app/blk-dir}"
CREATE_DIRS="${CREATE_DIRS:-/app/index-dir /app/rocksdb}"

RUN_AS="${RUN_AS:-$APP_UID:$APP_GID}"

ensure_dirs() {
    [ -n "$CREATE_DIRS" ] || return 0
    for dir in $CREATE_DIRS; do
        if [ ! -d "$dir" ]; then
            log "Creating $dir"
            mkdir -p "$dir"
        fi
    done
}

copy_index() {
    if [ -d "/app/blk-dir/index" ]; then
        log "Copying /app/blk-dir/index -> /app/index-dir"
        if [ ! -d "/app/index-dir" ]; then
            if is_root; then
                mkdir -p /app/index-dir
            else
                log "Error: /app/index-dir does not exist and cannot be created as non-root"
                exit 1
            fi
        fi
        rsync -a --delete /app/blk-dir/index/ /app/index-dir/
        log "Copy complete"
    else
        log "Source /app/blk-dir/index does not exist, skipping rsync"
    fi
}

fix_perms() {
    [ -n "$FIX_PERMS_DIRS" ] || return 0
    EXCLUDE_PARAMS=""
    for excl in $EXCLUDE_DIRS; do
        EXCLUDE_PARAMS="$EXCLUDE_PARAMS -not -path '$excl*'"
    done
    for dir in $FIX_PERMS_DIRS; do
        if [ -d "$dir" ]; then
            log "Fixing permissions on $dir -> $RUN_AS (excluding $EXCLUDE_DIRS)"
            eval "find '$dir' -mindepth 1 $EXCLUDE_PARAMS -exec chown $RUN_AS {} +"
        else
            log "Skip $dir (not a directory)"
        fi
    done
}

if is_root; then
    ensure_dirs
fi

copy_index

if is_root; then
    fix_perms
    log "Starting as $RUN_AS"
    exec gosu "$RUN_AS" "$@"
fi

exec "$@"
