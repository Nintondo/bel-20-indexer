#!/usr/bin/env bash
set -Eeuo pipefail

for value in "$SERVICE_NAME" "${COIN:-}" "${NETWORK:-}" "${SERVICE_ENVIRONMENT:-}"; do
  [[ -z "$value" || "$value" =~ ^[a-zA-Z0-9_-]+$ ]] || { echo 'Invalid service or environment segment'; exit 1; }
done
if [ "${MAINTENANCE_ONLY:-false}" != true ]; then
  [[ "$SERVICE_TAG" =~ ^[a-f0-9]{40}$ ]] || { echo 'Deploy requires a full immutable commit SHA'; exit 1; }
fi
for value in "$DOCKER_COMPOSE_FILE" "${SERVICE_COMPOSE_FILE:-}" "${COMPOSE_BASE_FILE:-}" "${CONFIG_TARGET_FILE:-}" "${COMPOSE_UPDATED_FILE:-}" "${CONFIG_UPDATED_FILE:-}" "${ENV_UPDATED_FILE:-}" "${CONF_DIR:-}" "${LIST_UPDATED_FILE:-}"; do
  [[ "$value" != /* && "$value" != *..* ]] || { echo 'Service file paths must be relative and stay inside the service'; exit 1; }
done

BASE_PATH="/opt/${COIN}-${NETWORK}"
SERVICE_PATH="${BASE_PATH}/services/${SERVICE_NAME}"
IMAGE_NAME="${IMAGE_NAME_OVERRIDE:-$SERVICE_NAME}"
IMAGE="${CI_REGISTRY}/${CI_REGISTRY_REPO}/${IMAGE_NAME}:${SERVICE_TAG}"
CONTAINER="${COIN}-${NETWORK}-${SERVICE_NAME}"
COMPOSE_SERVICE="${COMPOSE_SERVICE_NAME_OVERRIDE:-${COMPOSE_SERVICE:-$CONTAINER}}"
TARGET_COMPOSE="$SERVICE_PATH/${DOCKER_COMPOSE_FILE}"

compose() {
local command=(docker compose)
if [ -n "${COMPOSE_COMMAND_TIMEOUT:-}" ]; then command=(timeout "$COMPOSE_COMMAND_TIMEOUT" docker compose); fi
"${command[@]}" -f "$BASE_PATH/docker-compose.yml" "$@"
}

snapshot_files() {
backup_file "$TARGET_COMPOSE" || return 1
if [ -n "$ENV_UPDATED_FILE" ]; then backup_file "$SERVICE_PATH/.env" || return 1; fi
}

apply_files() {
sed -i "s|image: .*/${IMAGE_NAME}:.*|image: ${IMAGE}|" "$TARGET_COMPOSE"
if [ -n "$ENV_UPDATED_FILE" ]; then mv "$SERVICE_PATH/$ENV_UPDATED_FILE" "$SERVICE_PATH/.env"; fi
}

run_migrations() {
:
}

verify_configured_image() {
local configured
configured=$(compose config --format json | python3 -c 'import json,sys; print(json.load(sys.stdin)["services"][sys.argv[1]]["image"])' "$COMPOSE_SERVICE")
[ "$configured" = "$IMAGE" ]
}

verify_runtime() {
:
}

legacy_probe() {
docker exec -i "$CONTAINER" sh <<'LEGACY_PROBE'
awk '$2 ~ /:1F40$/ && $4 == "0A" {found=1} END {exit !found}' /proc/net/tcp /proc/net/tcp6
LEGACY_PROBE
}


# One persistent transaction spans remote readiness and the CI public probe.
# The directory also blocks a second deployment while recovery is unresolved.
umask 077
[[ "$DEPLOYMENT_ID" =~ ^[a-zA-Z0-9_-]+$ ]] || { echo 'Invalid deployment ID'; exit 1; }
[[ "$HEALTH_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] || { echo 'Invalid health timeout'; exit 1; }
test -d "$SERVICE_PATH"
cd "$BASE_PATH"
exec 9>"$SERVICE_PATH/.deploy.lock"
flock -w 5 9
TRANSACTION="$SERVICE_PATH/.deploy-transaction"

backup_file() {
  local target="$1" index
  [[ "$target" = /* ]] || { echo 'Backup target must be absolute'; return 1; }
  index=$(find "$TRANSACTION" -maxdepth 1 -name 'target-*' | wc -l) || return 1
  printf '%s' "$target" > "$TRANSACTION/target-$index" || return 1
  if [ -e "$target" ]; then
    cp -p "$target" "$TRANSACTION/file-$index" || return 1
  else
    touch "$TRANSACTION/absent-$index" || return 1
  fi
}

wait_ready() {
  local mode="$1" state='' deadline=$((SECONDS + HEALTH_TIMEOUT_SECONDS)) stable=0
  while [ "$SECONDS" -lt "$deadline" ]; do
    state=$(docker inspect -f '{{.State.Status}} {{if .State.Health}}{{.State.Health.Status}}{{else}}missing{{end}}' "$CONTAINER") || return 1
    case "$state" in
      'running healthy') return 0 ;;
      'running missing')
        if [ "$mode" = rollback ]; then
          # Older images do not yet expose /readyz or Docker HEALTHCHECK.
          # Check their existing RPC (nodes) or listening socket (applications).
          if legacy_probe; then
            stable=$((stable + 1))
            [ "$stable" -lt 3 ] || return 0
          else
            stable=0
          fi
        else
          echo 'New image must provide a Docker healthcheck'; return 1
        fi ;;
      'running unhealthy'|exited*|dead*) echo "Readiness failed: $state"; return 1 ;;
    esac
    sleep 3
  done
  echo "Readiness timed out: $state"
  return 1
}

restore_transaction() {
  local target marker index
  # Do not swallow errors or delete the only recovery copy on failure.
  for marker in "$TRANSACTION"/target-*; do
    [ -f "$marker" ] || continue
    target=$(cat "$marker")
    index=${marker##*/target-}
    if [ -f "$TRANSACTION/absent-$index" ]; then
      rm -f "$target" || return 1
    else
      cp -p "$TRANSACTION/file-$index" "$target" || return 1
    fi
  done
  compose config --quiet || return 1
  compose up -d --no-deps --force-recreate "$COMPOSE_SERVICE" || return 1
  [ "$(docker inspect -f '{{.Config.Image}}' "$CONTAINER")" = "$(cat "$TRANSACTION/previous-image")" ] || return 1
  wait_ready rollback || return 1
  if [ "$RELOAD_NGINX" = true ]; then
    docker exec nginx nginx -t || return 1
    docker exec nginx nginx -s reload || return 1
  fi
  touch "$TRANSACTION/restored"
  echo 'Previous image and service files restored; backups retained for review.'
}

owns_transaction() {
  [ -f "$TRANSACTION/id" ] && [ "$(cat "$TRANSACTION/id")" = "$DEPLOYMENT_ID" ]
}

case "$DEPLOY_PHASE" in
  rollback)
    if ! owns_transaction; then
      echo 'No transaction belonging to this run; nothing to roll back.'
      exit 0
    fi
    [ ! -f "$TRANSACTION/restored" ] || exit 0
    if ! restore_transaction; then
      echo "ROLLBACK FAILED; recovery files retained at $TRANSACTION" >&2
      exit 1
    fi
    exit 0 ;;
  commit)
    owns_transaction || { echo 'Deployment transaction is missing'; exit 1; }
    [ ! -f "$TRANSACTION/restored" ] || { echo 'Cannot commit a restored deployment'; exit 1; }
    [ "$(docker inspect -f '{{.Config.Image}}' "$CONTAINER")" = "$IMAGE" ]
    wait_ready "${READINESS_MODE:-deploy}"
    rm -rf "$TRANSACTION"
    echo 'Deployment committed after all readiness checks.'
    exit 0 ;;
  deploy) ;;
  *) echo 'Unknown deployment phase'; exit 1 ;;
esac

if [ -d "$TRANSACTION" ]; then
  # A successfully restored prior run can be archived on the next deployment.
  if [ -f "$TRANSACTION/restored" ]; then
    mv "$TRANSACTION" "$SERVICE_PATH/.deploy-restored-$(cat "$TRANSACTION/id")"
  else
    echo "Unresolved deployment at $TRANSACTION; recover it before deploying." >&2
    exit 1
  fi
fi
PREVIOUS_IMAGE=$(docker inspect -f '{{.Config.Image}}' "$CONTAINER")
[ -n "$PREVIOUS_IMAGE" ]
mkdir -m 700 "$TRANSACTION"
printf '%s' "$DEPLOYMENT_ID" > "$TRANSACTION/id"
printf '%s' "$PREVIOUS_IMAGE" > "$TRANSACTION/previous-image"
# A failed snapshot leaves the running service untouched.
if ! snapshot_files; then
  rm -rf "$TRANSACTION"
  echo 'Cannot snapshot service files; deployment has not started.' >&2
  exit 1
fi

deployment_exit() {
  local status=$?
  trap - EXIT INT TERM
  if [ "$status" -ne 0 ]; then
    echo 'Deployment failed; restoring previous service files and image.'
    if ! restore_transaction; then
      echo "ROLLBACK FAILED; recovery files retained at $TRANSACTION" >&2
    fi
  fi
  exit "$status"
}
trap deployment_exit EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
apply_files
compose config --quiet
verify_configured_image
compose pull "$COMPOSE_SERVICE"
run_migrations
compose up -d --no-deps --force-recreate "$COMPOSE_SERVICE"
[ "$(docker inspect -f '{{.Config.Image}}' "$CONTAINER")" = "$IMAGE" ]
wait_ready "${READINESS_MODE:-deploy}"
verify_runtime
if [ "$RELOAD_NGINX" = true ]; then
  docker exec nginx nginx -t
  docker exec nginx nginx -s reload
fi
trap - EXIT INT TERM
echo "Remote readiness passed; transaction remains pending at $TRANSACTION"
