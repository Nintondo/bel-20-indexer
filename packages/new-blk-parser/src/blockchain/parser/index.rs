use std::cmp::Ordering;
use std::time::Instant;
use std::{collections::BTreeMap, io::Read};

use super::*;

use byteorder::ReadBytesExt;
use indexmap::IndexMap;
use itertools::Itertools;
use rusty_leveldb::{DB, LdbIterator, Options};

const BLOCK_HAVE_DATA: u64 = 8;
const BLOCK_HAVE_UNDO: u64 = 16;
const BLOCK_VALID_RESERVED: u64 = 1;
const BLOCK_VALID_TREE: u64 = 2;
const BLOCK_VALID_CHAIN: u64 = 4;
const BLOCK_VALID_TRANSACTIONS: u64 = 3;
const BLOCK_VALID_SCRIPTS: u64 = 5;

const BLOCK_VALID_MASK: u64 = BLOCK_VALID_RESERVED | BLOCK_VALID_TREE | BLOCK_VALID_TRANSACTIONS | BLOCK_VALID_CHAIN | BLOCK_VALID_SCRIPTS;

const BLOCK_FAILED_VALID: u64 = 32;
const BLOCK_FAILED_CHILD: u64 = 64;
const BLOCK_FAILED_MASK: u64 = BLOCK_FAILED_VALID | BLOCK_FAILED_CHILD;
const BLOCK_HAVE_MWEB: u64 = 1 << 28;

/// Holds the index of longest valid chain
pub struct ChainIndex {
    max_height: u64,
    pub block_index: HashMap<u64, BlockIndexRecordSmall>,
    max_height_blk_index: HashMap<u64, u64>, // Maps blk_index to max_height found in the file
}

impl ChainIndex {
    pub fn new(options: &ChainOptions) -> Result<Self> {
        let path = &options.index_dir_path;
        let start = Instant::now();
        let block_index = path
            .as_ref()
            .map(|path| get_block_index(path, options.range, options.coin))
            .transpose()?
            .unwrap_or_default();
        tracing::trace!("Loaded block indexes from LevelDB in {}s", start.elapsed().as_secs_f64());
        let mut max_height_blk_index = HashMap::new();

        for (height, index_record) in &block_index {
            match max_height_blk_index.get(&index_record.blk_index) {
                Some(cur_height) if height > cur_height => {
                    max_height_blk_index.insert(index_record.blk_index, *height);
                }
                None => {
                    max_height_blk_index.insert(index_record.blk_index, *height);
                }
                _ => {}
            }
        }

        let max_known_height = block_index.keys().max().copied().unwrap_or_default();
        let max_height = match options.range.end {
            Some(height) if height < max_known_height => height,
            Some(_) | None => max_known_height,
        };

        Ok(Self {
            max_height,
            block_index,
            max_height_blk_index,
        })
    }

    /// Returns the `BlockIndexRecord` for the given height
    #[inline]
    pub fn get(&self, height: u64) -> Option<&BlockIndexRecordSmall> {
        self.block_index.get(&height)
    }

    /// Returns the maximum height known
    #[inline]
    pub const fn max_height(&self) -> u64 {
        self.max_height
    }

    /// Returns the maximum height that can be found in the given blk_index
    #[inline]
    pub fn max_height_by_blk(&self, blk_index: u64) -> u64 {
        *self.max_height_blk_index.get(&blk_index).unwrap()
    }
}

/// Holds the metadata where the block data is stored,
/// See https://bitcoin.stackexchange.com/questions/28168/what-are-the-keys-used-in-the-blockchain-leveldb-ie-what-are-the-keyvalue-pair
#[derive(Clone)]
pub struct BlockIndexRecord {
    pub block_hash: sha256d::Hash,
    pub blk_index: u64,
    pub data_offset: u64,
    height: u64,
    status: u64,
    prev_hash: sha256d::Hash,
}

pub struct BlockIndexRecordSmall {
    pub block_hash: sha256d::Hash,
    pub blk_index: u64,
    pub data_offset: u64,
}

impl From<BlockIndexRecord> for BlockIndexRecordSmall {
    fn from(value: BlockIndexRecord) -> Self {
        Self {
            blk_index: value.blk_index,
            block_hash: value.block_hash,
            data_offset: value.data_offset,
        }
    }
}

impl BlockIndexRecord {
    fn from(key: &[u8], values: &[u8], coin: CoinType) -> Result<Option<Self>> {
        let mut reader = Cursor::new(values);

        let block_hash: [u8; 32] = key.try_into().expect("leveldb: malformed blockhash");
        let _version = read_varint(&mut reader)?;
        let height = read_varint(&mut reader)?;
        let status = read_varint(&mut reader)?;
        let _tx_count = read_varint(&mut reader)?;

        let blk_index: u64 = if status & (BLOCK_HAVE_DATA | BLOCK_HAVE_UNDO) > 0 {
            read_varint(&mut reader)?
        } else {
            return Ok(None);
        };

        let mut data_offset: Option<u64> = None;
        let mut _undo_offset: Option<u64> = None;

        if status & BLOCK_HAVE_DATA > 0 {
            data_offset = Some(read_varint(&mut reader)?);
        }
        if status & BLOCK_HAVE_UNDO > 0 {
            _undo_offset = Some(read_varint(&mut reader)?);
        }

        if coin.has_mweb_extension_metadata() && status & BLOCK_HAVE_MWEB > 0 {
            skip_mweb_extension(&mut reader)?;
        }

        let block_header = reader.read_block_header()?;

        Ok(Some(BlockIndexRecord {
            block_hash: sha256d::Hash::from_byte_array(block_hash),
            height,
            status,
            blk_index,
            data_offset: data_offset.unwrap(),
            prev_hash: block_header.prev_hash,
        }))
    }
}

impl fmt::Debug for BlockIndexRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BlockIndexRecord")
            .field("block_hash", &self.block_hash)
            .field("height", &self.height)
            .field("status", &self.status)
            .field("n_file", &self.blk_index)
            .field("n_data_pos", &self.data_offset)
            .field("prev_hash", &self.prev_hash)
            .finish()
    }
}

fn try_build(per_height: &IndexMap<u64, Vec<BlockIndexRecord>>, heights: &[u64], start: BlockIndexRecord, min_height: u64) -> Option<BTreeMap<u64, BlockIndexRecord>> {
    let mut chain = BTreeMap::new();
    chain.insert(start.height, start.clone());
    let mut cur = start;

    // Walk backwards from start.height - 1 down to min_height
    for h in heights.iter().rev().cloned() {
        if h >= cur.height {
            continue;
        }
        if h < min_height {
            break;
        }
        let prevs = per_height.get(&h)?;
        if let Some(prev) = prevs.iter().find(|r| r.block_hash == cur.prev_hash) {
            chain.insert(h, prev.clone());
            cur = prev.clone();
        } else {
            return None; // failed to link
        }
    }
    Some(chain)
}

pub fn get_block_index(path: &Path, range: crate::utils::BlockHeightRange, coin: CoinType) -> Result<HashMap<u64, BlockIndexRecordSmall>> {
    let mut block_index = IndexMap::<u64, Vec<BlockIndexRecord>>::with_capacity(900_000);
    let mut db_iter = DB::open(path, Options::default())?.new_iter()?;
    let (mut key, mut value) = (vec![], vec![]);

    db_iter.seek(b"b");
    db_iter.prev();

    while db_iter.advance() {
        db_iter.current(&mut key, &mut value);

        if !is_block_index_record(&key) {
            break;
        }

        let Some(record) = BlockIndexRecord::from(&key[1..], &value, coin)? else {
            continue;
        };

        if record.height < range.start.saturating_sub(1) || record.height > range.end.unwrap_or(u64::MAX) {
            continue;
        }

        let level = record.status & BLOCK_VALID_MASK;
        if (record.status & BLOCK_HAVE_DATA) == 0 {
            continue;
        }
        if (record.status & BLOCK_FAILED_MASK) != 0 {
            continue;
        }
        if level < BLOCK_VALID_TRANSACTIONS {
            continue;
        }

        block_index.entry(record.height).or_default().push(record);
    }

    block_index.sort_unstable_keys();

    let heights: Vec<u64> = block_index.keys().cloned().collect();
    if heights.is_empty() {
        return Ok(HashMap::new());
    }
    let &top_height = heights.last().unwrap();

    let min_height = range.start.saturating_sub(1);

    // Try candidates at the top height until a full link is found
    let top_candidates = block_index.get(&top_height).cloned().unwrap_or_default();
    let mut sorted_top = top_candidates.clone();
    sorted_top.sort_by(|a, b| {
        let la = a.status & BLOCK_VALID_MASK;
        let lb = b.status & BLOCK_VALID_MASK;
        match lb.cmp(&la) {
            // higher first
            Ordering::Equal => match a.blk_index.cmp(&b.blk_index) {
                // lower file first
                Ordering::Equal => a.data_offset.cmp(&b.data_offset), // lower offset first
                o => o,
            },
            o => o,
        }
    });

    for cand in sorted_top {
        if let Some(chain) = try_build(&block_index, &heights, cand, min_height) {
            // Convert to expected map type
            let out = chain.into_iter().map(|(h, r)| (h, r.into())).collect();
            return Ok(out);
        }
    }

    // If we reach here, we couldn't link prev_hash consistently.
    anyhow::bail!("Failed to find previous block hash. -reindex-chainstate is required to proceed");
}

#[inline]
fn is_block_index_record(data: &[u8]) -> bool {
    *data.first().unwrap() == b'b'
}

fn read_varint(reader: &mut Cursor<&[u8]>) -> Result<u64> {
    let mut n = 0;
    loop {
        let ch_data = reader.read_u8()?;
        if n > u64::MAX >> 7 {
            panic!("size too large");
        }
        n = (n << 7) | (ch_data & 0x7F) as u64;
        if ch_data & 0x80 > 0 {
            if n == u64::MAX {
                panic!("size too large");
            }
            n += 1;
        } else {
            break;
        }
    }
    Ok(n)
}

fn skip_mweb_extension(reader: &mut Cursor<&[u8]>) -> Result<()> {
    // MWEB header fields
    read_varint(reader)?; // height

    let mut buf = [0u8; 32];
    reader.read_exact(&mut buf)?; // output root
    reader.read_exact(&mut buf)?; // kernel root
    reader.read_exact(&mut buf)?; // leafset root
    reader.read_exact(&mut buf)?; // kernel offset
    reader.read_exact(&mut buf)?; // stealth offset

    read_varint(reader)?; // output MMR size
    read_varint(reader)?; // kernel MMR size

    reader.read_exact(&mut buf)?; // hogex hash
    read_varint(reader)?; // mweb amount

    Ok(())
}
