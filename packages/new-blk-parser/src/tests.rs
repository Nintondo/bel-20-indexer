use crate::proto::header::BlockHeader;

use super::*;

fn test_block_id(height: u64) -> BlockId {
    BlockId {
        height,
        hash: sha256d::Hash::from_byte_array([height as u8; 32]),
    }
}

#[test]
fn startup_refuses_replaced_saved_tip() {
    let saved = test_block_id(100);
    assert!(validate_tip(saved, saved.hash).is_ok());
    assert!(validate_tip(saved, test_block_id(101).hash).is_err());
}

#[test]
fn test_reorg() {
    let blocks = [test_block_id(0), test_block_id(1), test_block_id(2), test_block_id(3), test_block_id(4), test_block_id(5)];
    let mut checkpoint = CheckPoint::from_block_ids(blocks).unwrap();

    let best_block_id = BlockId {
        height: 3,
        hash: sha256d::Hash::from_byte_array([6; 32]),
    };

    let mut last_hash = checkpoint.hash();

    let mut reorg_counter = 0;

    if best_block_id.hash != checkpoint.hash() {
        let best_height = best_block_id.height;

        while checkpoint.height() >= best_block_id.height {
            reorg_counter += 1;
            checkpoint = checkpoint.prev().unwrap();
            last_hash = checkpoint.hash();
            continue;
        }

        assert_eq!(reorg_counter, 3);
        assert_eq!(checkpoint.height(), best_height - 1);

        while checkpoint.height() < best_height {
            let next_height = checkpoint.height() + 1;
            let next_hash = blocks.get(next_height as usize - 1).unwrap().hash;
            checkpoint = checkpoint.insert(BlockId {
                height: next_height,
                hash: next_hash,
            });

            check_ordering(
                &mut last_hash,
                &Hashed {
                    hash: blocks.get(checkpoint.height() as usize - 1).map(|x| x.hash).unwrap_or(sha256d::Hash::all_zeros()),
                    value: BlockHeader {
                        bits: 0,
                        merkle_root: sha256d::Hash::all_zeros(),
                        nonce: 0,
                        prev_hash: checkpoint.hash(),
                        timestamp: 0,
                        version: 0,
                    },
                },
            );
        }
    }

    assert_eq!(checkpoint.height(), best_block_id.height);
}
