# Deployment and recovery

Deployment actions and templates are checked out at `github.workflow_sha`,
independently of the image being deployed or rolled back. Rust application
dependencies are committed in Cargo.lock and built with `--locked`. CI enables
application unit tests in the builder without adding them to the runtime image.

Deployments use immutable full commit SHA image tags and restart only the selected
Compose service (`--no-deps`). Node data, application data and database volumes
are not restored or deleted by application rollback.

The SSH action keeps a private `.deploy-transaction` directory under the service
directory until Docker health, runtime checks and any configured
`PUBLIC_HEALTH_URL` have passed. A failed external probe invokes a separate SSH
recovery step. Restoration includes every staged service file, including `.env`
and its prior absence, node configs, pusher RPC lists and frontend maintenance.
The running image is verified against the expected Compose image.

Recovery failures fail the workflow and preserve the recovery files. A pending
transaction blocks subsequent deployments on the host, including other workflow
runs. After successful recovery the files remain for inspection; the next run
archives them as `.deploy-restored-<id>`. These directories can contain credentials
and must stay private. Remove old restored archives only after reviewing recovery.
Do not remove an unresolved transaction to bypass the deployment guard.

For a failed rollback, inspect the workflow log and the service transaction on the
host, correct the reported cause, and rerun the rollback phase with the **same**
deployment ID and original action inputs. A different run never restores another
run's files. If SSH is interrupted before confirmation, the pending transaction
remains available for this recovery. Database migrations are not downgraded;
release schema changes must remain compatible with the previous application.

New images must have Docker HEALTHCHECK. Existing legacy images may lack it:
node recovery verifies RPC using the shipped CLI; frontend recovery verifies
its existing readiness endpoint; other applications verify their listening
socket for three samples. The legacy socket probe is a limited recovery check,
not a dependency readiness check. Modern image recovery requires healthy status.

`PUBLIC_HEALTH_URL` is optional and checked from the CI runner. If absent, there
is no external HTTP gate. Configure it in each GitHub Actions environment when
the service has a public readiness endpoint. Health for a node means responsive
RPC, not completed chain synchronization. Track IBD and tip lag separately.

## Validation and rollout

Deployment checks run on GitHub-hosted runners with read-only permissions and
without registry or server credentials. They validate workflows and shell,
exercise injected deployment/recovery failures, test the real Compose transaction
with disposable fixture images, and build the actual image at the PR head.
The fixture Compose test verifies orchestration, not application dependencies.

Before production rollout, require the deployment-contracts and image-build checks,
deploy one eligible testnet environment, verify public readiness and dependency
behavior, and inspect node/indexer progress. Then advance remaining eligible
testnet environments. Keep mainnet deployment manual with `DEPLOY_MAINNET`.
Doge testnet is excluded from this rollout. Legacy Bells is an independent reserve
and stays unchanged. The active Bells mainnet image remains
`2cd82c7741d7e578c3597e9ade09e94daf31cb0e` until a separately chosen node release.
