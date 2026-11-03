# Holdings synchronization API (version 1)

The `/sync` routes export current Bells, Doge and Pepe token balances for the
wallet overview service. Existing token validation/accounting and `/events`
payloads are unchanged. Wire types live in `packages/holdings-sync-protocol`.

## Snapshot import

1. `POST /sync/snapshot` returns `{id, status}`. Status contains
   `protocol_version`, `chain`, `network`, and `tip: {height, hash}`.
2. `GET /sync/snapshot/{id}?limit=1000&cursor=...` returns
   `{items, next_cursor}`. Omit the cursor on the first page. A null cursor
   marks the end; each supplied cursor is exclusive. Limits are 1..1000.
3. `DELETE /sync/snapshot/{id}` releases the source snapshot (204).

Every page reads the same RocksDB snapshot, captured at a committed block
boundary. Writers continue processing blocks while the snapshot is read.
Sessions expire after 15 minutes without a page request; cleanup runs every
30 seconds. At most four sessions may exist simultaneously. Snapshot sessions
are in memory and do not survive source restarts; clients then restart import.
An expired/closed session returns 410. Invalid limits/cursors return 400.

Each balance contains `address`, original `tick`, `token_id` (genesis inscription),
`decimals`, `available`, `transferable`, and `transfers_count`. Amounts are exact
nonnegative decimal strings. Total ownership is available plus transferable.
Snapshot pages omit zero holdings and scripts without a standard address.

Validate the snapshot anchor with `/sync/block/{height}` before publishing an
import, since it may have been replaced while the snapshot was being read.

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

Balances are absolute post-block values. Zeroes are included to clear holdings.
Blocks without balance changes are included so clients can advance contiguous
checkpoints. Limits are 1..100. The server verifies that `after_hash` is still
canonical and reads the whole page under the block/reorg publication lock.

- 409: checkpoint is no longer canonical. Roll back to a common ancestor.
- 410: requested block predates this installation's replay log. Import a snapshot.
- 503: no committed tip is available yet, or this chain is unsupported.

`GET /sync/block/{height}` returns the canonical `{height, hash}`, or 404 if
unavailable. `GET /sync/status` returns current protocol/chain/network/tip.
Replay starts with blocks processed after this feature is installed; the existing
database does not need a reindex. Replay records currently have no automatic
retention limit.

This upgrade adds a `SYNC_BLOCKS` column family. Earlier binaries that open an
explicit column-family list cannot open that upgraded database; a binary rollback
requires restoring a compatible database or removing that export-only column
family while the indexer is stopped.

Balances, metadata, original history, replay records, and block markers are
written in one RocksDB batch. Reorg rollback atomically restores each block and
removes its replay record. Consumers should persist their balance changes,
undo values and checkpoint together too. Client rollback below its snapshot
anchor requires importing a fresh snapshot.

## Live notifications

`GET /sync/events` is an SSE stream containing `event: sync` notifications:
`ready`, `new_block`, and `reorg`. New-block notifications are emitted after
commit. Unlike `/events`, it does not serialize individual history events.
Keepalive comments keep idle connections open. A lagging subscriber disconnects
and must reconnect and replay from its durable checkpoint.

Notifications trigger catch-up; they are not a durable data delivery mechanism.
Poll the replay endpoint periodically as well, including after restarts and
reconnections. Snapshot and sync routes use blocking workers for RocksDB reads.

## Validation

`cargo test --workspace` includes snapshot paging under concurrent changes,
zero-balance export, committed empty-block replay, orphan removal, and replay-gap
responses. These tests run without a blockchain node.
