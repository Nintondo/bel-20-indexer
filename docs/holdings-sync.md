# Holdings synchronization API (version 2)

The `/sync` routes export current Bells, Doge and Pepe token balances for the wallet
overview service. Existing token validation/accounting and `/events` payloads are
unchanged. Wire types live in `packages/holdings-sync-protocol`.

## Access and configuration

Every `/sync` route requires `Authorization: Bearer <SYNC_TOKEN>`. With no `SYNC_TOKEN`,
all routes return 404 and no export data is generated. Set a random secret of at least
32 characters in each source environment and the matching `BELLS_SYNC_TOKEN`,
`DOGE_SYNC_TOKEN`, or `PEPE_SYNC_TOKEN` in the consumer. Use HTTPS or a private network.
Deployment templates accept the environment's `SYNC_TOKEN` secret; no secret is
committed.

Replay retention is a fixed 10,000 blocks; legacy `SYNC_RETENTION_BLOCKS` settings are
ignored, including when exports are disabled. The `/docs/private/api.json` OpenAPI
document includes the sync routes and bearer security scheme. Wire identities use
strings so unknown future chains/networks can be decoded; clients must still validate
their configured identities.

## Snapshot import

1. `POST /sync/snapshot` returns `{id, status}`. Status contains
   `protocol_version`, `chain`, `network`, `tip: {height, hash}`, `replay_start`,
   and `catching_up`. Snapshot creation returns 503 until the ordered node-tip marker is committed.
2. `GET /sync/snapshot/{id}?limit=1000&cursor=...` returns
   `{items, next_cursor}`. Omit the cursor on the first page. A null cursor
   marks the end; each supplied cursor is exclusive. Limits are 1..1000. Each page scans at most 5,000 storage rows, including zero holdings and scripts without addresses. An empty `items` array with a non-null cursor means continue.
3. `DELETE /sync/snapshot/{id}` releases the source snapshot (204).

Every page reads the same RocksDB snapshot, captured at a committed block boundary.
Writers continue processing blocks while the snapshot is read. Sessions expire after 15
minutes without a page request or one hour total; cleanup runs every 30 seconds on a
blocking worker. At most four sessions may exist simultaneously. Snapshot sessions are
in memory and do not survive source restarts or falling more than 30 blocks behind;
clients then restart import. An expired/closed session returns 410. Invalid
limits/cursors return 400. Capacity exhaustion returns 429. Read/decoding/metadata
failures return 500 and leave the session intact: retry the same page rather than
restart import.

Each balance contains `address`, `account_id` (full source script hash), original
`tick`, `token_id` (genesis inscription), `decimals`, `available`, `transferable`, and
`transfers_count`. Amounts are exact nonnegative decimal strings. Total ownership is
available plus transferable. Snapshot pages omit zero holdings and scripts without a
standard address. Distinct scripts that decode to the same address keep separate
`account_id` values; consumers must not overwrite them by address/token alone.

Validate the snapshot anchor by requesting `/sync/blocks` after its height/hash before
publishing an import. If the source is temporarily behind or `catching_up` is true,
retain the import and wait.

## Restart catch-up

`GET /sync/blocks?after_height=H&after_hash=HASH&limit=50`

Returns `{status, blocks}`. Each block contains:

```json
{
  "block": {"height":123,"hash":"..."},
  "prev_hash":"...",
  "created":1700000000,
  "balances":[]
}
```

Balances are absolute post-block values. Zeroes are included to clear holdings. Blocks
without balance changes are included so clients can advance contiguous checkpoints.
Limits are 1..100. The server verifies that `after_hash` is still canonical. It captures
status and a RocksDB snapshot under the block/reorg publication lock, then releases the
lock before reading and encoding the page. Hash comparison accepts either hex case.

- 200 with an empty page: source has not reached the checkpoint yet or is bulk
  indexing. Wait; do not roll back.
- 409: checkpoint height has been indexed and its hash differs. Roll back.
- 410: checkpoint predates the retained replay boundary. Import a snapshot.
- 500: storage/metadata/decoding failure. Retry and report the source error.

`GET /sync/block/{height}` returns the canonical `{height, hash}`, 503 while the tip is
unverified/recovering or the source has not reached that height, or 404 if it has no
metadata for an already-indexed height. `GET /sync/status` returns current
protocol/chain/network/tip. Replay starts at installation. Blocks within 10,000 blocks
of the observed node tip retain changes even during catch-up. Only blocks outside that
window skip historical export work; a restart or 30-block lag does not by itself expire
a checkpoint. A persistent replay-start marker separates missing history from unchanged
balances: only blocks with balance changes have records. Empty-block responses use
existing block metadata. The replay boundary advances atomically with balances. Expired
records are cleaned with range tombstones every 30 seconds outside the publication lock.
Pages have a hard 16 MiB serialized limit and are serialized once. A single block too
large for replay returns 410 so the consumer imports a snapshot.

On-disk replay records use an independent, versioned storage schema. Updating the wire
contract does not reinterpret old postcard records. Installation over the first
prototype resets its disposable replay log and preserves token balances; consumers
outside the new boundary import a snapshot.

Export tables and the persisted chain/network identity use namespaced keys in the
existing default column family. Prototype `SYNC_BLOCKS` and `SYNC_STATE` column families
are removed at initialization after resetting their disposable log. Earlier binaries can
reopen this database during deployment rollback. Reusing the database with a different
chain or network is rejected.

Set `BLK_DIR` and `INDEX_DIR` together, or leave both unset/empty for RPC-only indexing.
Partial configuration is rejected, and a local-file checkpoint cannot advance beyond the
last block actually parsed. A stale copied index can resume from the RPC-verified saved
tip.

Startup verifies the saved tip hash against the node before processing or publishing
readiness. A mismatch (for example, a crash followed by a reorg while offline) fails
closed with a reindex-required error. This protects consumers; it does not provide
durable undo recovery for pre-existing indexer data. Parser failure, shutdown, and live
rollback revoke readiness and invalidate snapshot sessions before undo or flush begins.
Canonical block lookup returns 503 throughout recovery.

Balances, metadata, original history, replay records, and block markers are written in
one RocksDB batch. If preparing the export log fails, balances and a replay gap marker
commit atomically, forcing consumers to resnapshot instead of silently skipping changes.
Reorg rollback atomically restores each block and removes its replay record. Consumers
should persist their balance changes, undo values and checkpoint together too. Client
rollback below its snapshot anchor requires importing a fresh snapshot.

## Live notifications

`GET /sync/events` is an SSE stream containing `event: sync` notifications: `ready`,
`new_block`, and `reorg`. New-block notifications are emitted after commit. Unlike
`/events`, it does not serialize individual history events. Keepalive comments keep idle
connections open. A lagging subscriber disconnects and must reconnect and replay from
its durable checkpoint. An ordered parser tip marker also clears initial recovery status
when an already-current source restarts without a new block arriving.

Notifications trigger catch-up; they are not a durable data delivery mechanism. Poll the
replay endpoint periodically as well, including after restarts and reconnections.
Snapshot and sync routes use blocking workers for RocksDB reads. Four export permits
cover both blocking work and response bodies; extra work is rejected immediately with
429. SSE has a separate 16-connection cap and releases permits on disconnect. Expensive
reads never queue behind an unbounded semaphore waiter list.

## Validation

`cargo test --workspace` covers snapshot isolation, authentication, error codes,
changed-balance replay, reorg/restore_all balance consistency, temporary tip regression,
sparse retention, legacy-log reset and contract roundtrips. These tests run without a
blockchain node.

The overview service keeps its local mirror because its API needs one ordered,
aggregated page across addresses and chains; direct address fan-out would still need a
cross-source pagination/aggregation mechanism on every request.