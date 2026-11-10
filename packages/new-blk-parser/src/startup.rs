use super::*;

pub fn validate_local_paths(blocks: Option<&str>, index: Option<&str>) -> anyhow::Result<()> {
    let present = |path: Option<&str>| path.is_some_and(|value| !value.trim().is_empty());
    anyhow::ensure!(
        present(blocks) == present(index),
        "Set BLK_DIR and INDEX_DIR together, or leave both empty for RPC-only indexing"
    );
    Ok(())
}

pub(super) fn validate_tip(stored: BlockId, canonical: sha256d::Hash) -> anyhow::Result<()> {
    anyhow::ensure!(
        stored.hash == canonical,
        "Hash mismatch at height {}: stored {}, node {}",
        stored.height,
        stored.hash,
        canonical
    );
    Ok(())
}

pub(super) fn validate_local_checkpoint(local: BlockId, emitted: BlockId, saved: BlockId) -> anyhow::Result<()> {
    let same = |a: BlockId, b: BlockId| a.height == b.height && a.hash == b.hash;
    anyhow::ensure!(
        same(local, emitted) || (local.height < emitted.height && same(emitted, saved)),
        "Local index tip differs from parsed blocks; check BLK_DIR/INDEX_DIR"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_checkpoint_cannot_skip_blocks_but_may_lag_saved_tip() {
        let id = |height, byte| BlockId {
            height,
            hash: sha256d::Hash::from_byte_array([byte; 32]),
        };
        assert!(validate_local_checkpoint(id(100, 1), id(100, 1), id(100, 1)).is_ok());
        assert!(validate_local_checkpoint(id(99, 2), id(100, 1), id(100, 1)).is_ok());
        assert!(validate_local_checkpoint(id(101, 3), id(100, 1), id(100, 1)).is_err());
        assert!(validate_local_checkpoint(id(100, 2), id(100, 1), id(100, 1)).is_err());
        assert!(validate_local_checkpoint(id(100, 1), id(101, 3), id(100, 1)).is_err());
    }

    #[test]
    fn local_data_and_index_must_be_paired() {
        for pair in [(None, None), (Some(""), Some("")), (Some(" "), None), (Some("/blocks"), Some("/index"))] {
            assert!(validate_local_paths(pair.0, pair.1).is_ok());
        }
        for pair in [(Some("/blocks"), None), (None, Some("/index")), (Some(""), Some("/index"))] {
            assert!(validate_local_paths(pair.0, pair.1).is_err());
        }
    }
}
