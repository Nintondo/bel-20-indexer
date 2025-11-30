use std::cmp::Ordering;
use std::time::Instant;
use std::{collections::BTreeMap, io::Read};

use super::*;

use byteorder::ReadBytesExt;
use indexmap::IndexMap;
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

/// Holds the index of a valid, contiguous chain selected by maximum cumulative work
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
    bits: u32,
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

        let block_hash: [u8; 32] = key
            .try_into()
            .map_err(|_| anyhow::anyhow!("leveldb: malformed blockhash"))?;
        let _version = read_varint(&mut reader)?;
        let height = read_varint(&mut reader)?;
        let status = read_varint(&mut reader)?;
        let _tx_count = read_varint(&mut reader)?;

        // We only care about blocks that actually have blk*.dat data.
        if (status & BLOCK_HAVE_DATA) == 0 {
            return Ok(None);
        }

        // Now we know nFile is present.
        let blk_index: u64 = read_varint(&mut reader)?;

        // And because HAVE_DATA is set, nDataPos is present too.
        let data_offset: u64 = read_varint(&mut reader)?;

        // Undo offset is optional.
        if status & BLOCK_HAVE_UNDO > 0 {
            let _undo_offset: u64 = read_varint(&mut reader)?;
            let _ = _undo_offset;
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
            data_offset,
            bits: block_header.bits,
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
            .field("bits", &self.bits)
            .field("prev_hash", &self.prev_hash)
            .finish()
    }
}

// Note: legacy height-based try_build removed; replaced by hash-indexed backtracking.

pub fn get_block_index(path: &Path, range: crate::utils::BlockHeightRange, coin: CoinType) -> Result<HashMap<u64, BlockIndexRecordSmall>> {
    let mut block_index = IndexMap::<u64, Vec<BlockIndexRecord>>::with_capacity(900_000);
    let mut by_hash = HashMap::<sha256d::Hash, BlockIndexRecord>::with_capacity(900_000);
    let mut db_iter = DB::open(path, Options::default())?.new_iter()?;

    db_iter.seek(b"b");
    trace!(target: "blkindex", "Scanning block index at {}", path.display());

    while db_iter.valid() {
        let Some((key, value)) = db_iter.current() else {
            break;
        };

        if !is_block_index_record(&key) {
            break;
        }

        if let Some(record) = BlockIndexRecord::from(&key[1..], &value, coin)? {
            // Do not filter out parents early; only apply upper-bound filter here.
            if record.height <= range.end.unwrap_or(u64::MAX) {
                let level = record.status & BLOCK_VALID_MASK;
                if (record.status & BLOCK_HAVE_DATA) != 0
                    && (record.status & BLOCK_FAILED_MASK) == 0
                    && level >= BLOCK_VALID_TRANSACTIONS
                {
                    // Store into hash index and per-height index
                    by_hash.insert(record.block_hash, record.clone());
                    block_index.entry(record.height).or_default().push(record);
                }
            }
        }

        if !db_iter.advance() {
            break;
        }
    }

    block_index.sort_unstable_keys();
    trace!(target: "blkindex", "Scanned {} records over {} heights", by_hash.len(), block_index.len());

    let heights: Vec<u64> = block_index.keys().cloned().collect();
    if heights.is_empty() {
        return Ok(HashMap::new());
    }
    let min_height = range.start.saturating_sub(1);
    // Compute cumulative log-work per block in ascending height order
    let mut cw_log2 = HashMap::<sha256d::Hash, f64>::with_capacity(by_hash.len());
    for h in &heights {
        if let Some(records) = block_index.get(h) {
            for rec in records {
                let parent_cw = cw_log2.get(&rec.prev_hash).copied().unwrap_or(0.0);
                let proof = block_proof_log2(rec.bits).unwrap_or(0.0);
                cw_log2.insert(rec.block_hash, parent_cw + proof);
            }
        }
    }

    // Sort candidate tips by cumulative work (desc), tie-breakers: height desc, validity desc, then disk order
    let mut candidates: Vec<(sha256d::Hash, f64)> = cw_log2.iter().map(|(h, w)| (*h, *w)).collect();
    candidates.sort_by(|(ha, wa), (hb, wb)| {
        match wb.partial_cmp(wa).unwrap_or(Ordering::Equal) {
            Ordering::Equal => {
                let a = by_hash.get(ha).unwrap();
                let b = by_hash.get(hb).unwrap();
                match b.height.cmp(&a.height) {
                    Ordering::Equal => {
                        let la = a.status & BLOCK_VALID_MASK;
                        let lb = b.status & BLOCK_VALID_MASK;
                        match lb.cmp(&la) {
                            Ordering::Equal => match a.blk_index.cmp(&b.blk_index) {
                                Ordering::Equal => a.data_offset.cmp(&b.data_offset),
                                o => o,
                            },
                            o => o,
                        }
                    }
                    o => o,
                }
            }
            o => o,
        }
    });

    if let Some((best_hash, best_work)) = candidates.first().copied() {
        if let Some(best) = by_hash.get(&best_hash) {
            trace!(target: "blkindex", "Best tip candidate: height={} work_log2={:.6} status={} file={} offset={}", best.height, best_work, best.status, best.blk_index, best.data_offset);
        }
    }

    // Try candidates by work until a fully linked chain down to min_height is found
    for (h, _w) in candidates {
        let start = by_hash.get(&h).unwrap();
        if let Some(chain) = try_build_by_hash(&by_hash, start, min_height) {
            let chain_start = chain.keys().next().copied().unwrap_or(0);
            let chain_tip = chain.keys().last().copied().unwrap_or(0);
            trace!(target: "blkindex", "Built chain back from tip height {} down to {} ({} entries)", chain_tip, chain_start, chain.len());

            // Drop the start-1 sentinel from the public map
            let out = chain
                .into_iter()
                .filter(|(h, _)| *h >= range.start)
                .map(|(h, r)| (h, r.into()))
                .collect();
            return Ok(out);
        }
        else {
            let st = by_hash.get(&h).unwrap();
            trace!(target: "blkindex", "Candidate failed to link: tip height {} hash {}", st.height, st.block_hash);
        }
    }

    anyhow::bail!("Failed to build a contiguous chain (missing ancestors or index corruption)");
}

#[inline]
fn is_block_index_record(data: &[u8]) -> bool {
    matches!(data.first(), Some(b'b'))
}

fn read_varint(reader: &mut Cursor<&[u8]>) -> Result<u64> {
    let mut n: u64 = 0;
    loop {
        let ch = reader.read_u8()?;
        if n > (u64::MAX >> 7) {
            anyhow::bail!("compact int too large");
        }
        n = (n << 7) | (ch & 0x7F) as u64;
        if (ch & 0x80) != 0 {
            if n == u64::MAX {
                anyhow::bail!("compact int too large");
            }
            n += 1;
        } else {
            break Ok(n);
        }
    }
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

fn try_build_by_hash(
    by_hash: &HashMap<sha256d::Hash, BlockIndexRecord>,
    start: &BlockIndexRecord,
    min_height: u64,
) -> Option<BTreeMap<u64, BlockIndexRecord>> {
    let mut chain = BTreeMap::new();
    chain.insert(start.height, start.clone());
    let mut cur = start.clone();

    while cur.height > min_height {
        let prev = by_hash.get(&cur.prev_hash)?;
        if prev.height + 1 != cur.height {
            return None;
        }
        chain.insert(prev.height, prev.clone());
        cur = prev.clone();
    }
    Some(chain)
}

/// Converts compact difficulty target (nBits) to log2(target).
/// Returns None for negative/invalid representations.
#[inline]
fn compact_target_to_log2(bits: u32) -> Option<f64> {
    let exp = (bits >> 24) as i32;
    let mant = bits & 0x007f_ffff; // 23-bit mantissa

    // Negative targets are invalid in this context
    if (bits & 0x0080_0000) != 0 {
        return None;
    }
    if mant == 0 {
        return None;
    }

    let log2_mant = (mant as f64).log2();
    let shift = exp - 3; // target = mantissa * 2^(8*(exp-3)) approximately
    Some(log2_mant + 8.0 * (shift as f64))
}

/// Approximate block proof as log2(work) using 256 - log2(target).
/// This tracks relative ordering of cumulative work without bigints.
#[inline]
fn block_proof_log2(bits: u32) -> Option<f64> {
    compact_target_to_log2(bits).map(|log2_target| 256.0 - log2_target)
}
