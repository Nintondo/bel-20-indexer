use super::*;
use std::time::Instant;

use bellscoin::{OutPoint, Witness, consensus::Decodable};
use byteorder::{LittleEndian, ReadBytesExt};
use proto::{
    MerkleBranch,
    block::{AuxPowExtension, Block},
    header::BlockHeader,
    tx::{RawTx, TxInput, TxOutput},
    varuint::VarUint,
};

/// Trait for structured reading of blockchain data
pub trait BlockchainRead: Read {
    #[inline]
    fn read_256hash(&mut self) -> Result<[u8; 32]> {
        let mut arr = [0u8; 32];
        self.read_exact(arr.borrow_mut())?;
        Ok(arr)
    }

    #[inline]
    fn read_u8_vec(&mut self, count: u32) -> Result<Vec<u8>> {
        let mut arr = vec![0u8; count as usize];
        self.read_exact(arr.borrow_mut())?;
        Ok(arr)
    }

    /// Reads a block as specified here: https://en.bitcoin.it/wiki/Protocol_specification#block
    fn read_block(&mut self, size: u32, coin: CoinType) -> Result<Block> {
        use crate::timing::BLOCK_READ_METRICS;
        let header_start = Instant::now();
        let header = self.read_block_header()?;
        BLOCK_READ_METRICS.record_header(header_start.elapsed());
        // Parse AuxPow data if present

        let aux_pow_extension = if coin.uses_aux_pow() && header.version & (1 << 8) != 0 {
            let start = Instant::now();
            let aux = Some(self.read_aux_pow_extension(coin)?);
            BLOCK_READ_METRICS.record_auxpow(start.elapsed());
            aux
        } else {
            None
        };

        let tx_count = VarUint::read_from(self)?;
        let txs_start = Instant::now();
        let txs = self.read_txs(tx_count.value, coin)?;
        BLOCK_READ_METRICS.record_tx_decode(txs_start.elapsed(), tx_count.value);
        Ok(Block::new(size, header, aux_pow_extension, tx_count, txs))
    }

    fn read_block_header(&mut self) -> Result<BlockHeader> {
        let version = self.read_u32::<LittleEndian>()?;
        let prev_hash = sha256d::Hash::from_byte_array(self.read_256hash()?);
        let merkle_root = sha256d::Hash::from_byte_array(self.read_256hash()?);
        let timestamp = self.read_u32::<LittleEndian>()?;
        let bits = self.read_u32::<LittleEndian>()?;
        let nonce = self.read_u32::<LittleEndian>()?;

        Ok(BlockHeader {
            version,
            prev_hash,
            merkle_root,
            timestamp,
            bits,
            nonce,
        })
    }

    fn read_txs(&mut self, tx_count: u64, coin: CoinType) -> Result<Vec<RawTx>> {
        (0..tx_count).map(|_| self.read_tx(coin)).collect()
    }

    /// Reads a transaction as specified here: https://en.bitcoin.it/wiki/Protocol_specification#tx
    fn read_tx(&mut self, coin: CoinType) -> Result<RawTx> {
        let mut flags = 0u8;
        let version = self.read_u32::<LittleEndian>()?;

        // Parse transaction inputs and check if this transaction contains segwit data
        let mut in_count = VarUint::read_from(self)?;
        if in_count.value == 0 {
            flags = self.read_u8()?;
            in_count = VarUint::read_from(self)?
        }
        let mut inputs = self.read_tx_inputs(in_count.value)?;

        // Parse transaction outputs
        let out_count = VarUint::read_from(self)?;
        let outputs = self.read_tx_outputs(out_count.value)?;

        // Check if the witness flag is present
        if flags & 1 > 0 {
            for input in inputs.iter_mut() {
                input.witness = Witness::consensus_decode(self)?;
            }
        }
        let locktime = self.read_u32::<LittleEndian>()?;
        let tx = RawTx {
            version,
            in_count,
            inputs,
            out_count,
            outputs,
            locktime,
            coin,
        };
        Ok(tx)
    }

    fn read_tx_outpoint(&mut self) -> Result<OutPoint> {
        let txid = sha256d::Hash::from_byte_array(self.read_256hash()?);
        let index = self.read_u32::<LittleEndian>()?;

        Ok(OutPoint { txid: txid.into(), vout: index })
    }

    fn read_tx_inputs(&mut self, input_count: u64) -> Result<Vec<TxInput>> {
        let mut inputs = Vec::with_capacity(input_count as usize);
        for _ in 0..input_count {
            let outpoint = self.read_tx_outpoint()?;
            let script_len = VarUint::read_from(self)?;
            let script_sig = self.read_u8_vec(script_len.value as u32)?;
            let seq_no = self.read_u32::<LittleEndian>()?;
            inputs.push(TxInput {
                outpoint,
                script_len,
                script_sig,
                seq_no,
                witness: Witness::default(),
            });
        }
        Ok(inputs)
    }

    fn read_tx_outputs(&mut self, output_count: u64) -> Result<Vec<TxOutput>> {
        let mut outputs = Vec::with_capacity(output_count as usize);
        for _ in 0..output_count {
            let value = self.read_u64::<LittleEndian>()?;
            let script_len = VarUint::read_from(self)?;
            let script_pubkey = self.read_u8_vec(script_len.value as u32)?;
            outputs.push(TxOutput { value, script_len, script_pubkey });
        }
        Ok(outputs)
    }

    /// Reads a merkle branch as specified here https://en.bitcoin.it/wiki/Merged_mining_specification#Merkle_Branch
    /// This is mainly used for merged mining (AuxPoW).
    fn read_merkle_branch(&mut self) -> Result<MerkleBranch> {
        let branch_length = VarUint::read_from(self)?;
        let hashes = (0..branch_length.value).map(|_| self.read_256hash()).collect::<Result<Vec<[u8; 32]>>>()?;
        let side_mask = self.read_u32::<LittleEndian>()?;
        Ok(MerkleBranch::new(hashes, side_mask))
    }

    /// Reads the additional AuxPow fields as specified here https://en.bitcoin.it/wiki/Merged_mining_specification#Aux_proof-of-work_block
    fn read_aux_pow_extension(&mut self, coin: CoinType) -> Result<AuxPowExtension> {
        let coinbase_tx = self.read_tx(coin)?;
        let block_hash = sha256d::Hash::from_byte_array(self.read_256hash()?);

        let coinbase_branch = self.read_merkle_branch()?;
        let blockchain_branch = self.read_merkle_branch()?;

        let parent_block = self.read_block_header()?;

        Ok(AuxPowExtension {
            coinbase_tx,
            block_hash,
            coinbase_branch,
            blockchain_branch,
            parent_block,
        })
    }
}

/// All types that implement `Read` and `Seek` get methods defined in `BlockchainRead`
/// for free.
impl<R: Read + Seek + ?Sized> BlockchainRead for R {}

/// Reader that XORs the data with a given key.
/// The block storage data is encrypted with a simple XOR operation
/// since Bitcoin Core 28.0.
/// See https://github.com/bitcoin/bitcoin/pull/28052
pub struct XorReader<R> {
    reader: R,
    xor_key: Option<Vec<u8>>,
    absolute_pos: u64,
}

impl<R: Seek + Read> XorReader<R> {
    pub fn new(reader: R, xor_key: Option<Vec<u8>>) -> XorReader<R> {
        Self { reader, xor_key, absolute_pos: 0 }
    }
}

impl<R: Read> Read for XorReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.reader.read(buf)?;
        if let Some(ref xor_key) = self.xor_key {
            use crate::timing::BLOCK_READ_METRICS;
            let key_len = xor_key.len();
            if key_len > 0 {
                let start = Instant::now();
                let mut processed = 0;
                let mut key_offset = (self.absolute_pos as usize) % key_len;
                let buf = &mut buf[..n];

                while processed < n {
                    let run_len = std::cmp::min(key_len - key_offset, n - processed);
                    let key_slice = &xor_key[key_offset..key_offset + run_len];
                    xor_with_key(&mut buf[processed..processed + run_len], key_slice);
                    processed += run_len;
                    key_offset = if key_offset + run_len == key_len { 0 } else { key_offset + run_len };
                }
                BLOCK_READ_METRICS.record_xor(start.elapsed());
            }
        }
        self.absolute_pos += n as u64;
        Ok(n)
    }
}

impl<R: Seek> Seek for XorReader<R> {
    fn seek(&mut self, pos: io::SeekFrom) -> io::Result<u64> {
        self.absolute_pos = self.reader.seek(pos)?;
        Ok(self.absolute_pos)
    }
}

#[inline]
fn xor_with_key(dst: &mut [u8], key: &[u8]) {
    debug_assert_eq!(dst.len(), key.len());
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse2") {
            unsafe {
                xor_with_key_sse2(dst, key);
            }
            return;
        }
    }
    xor_with_key_fallback(dst, key);
}

#[inline]
fn xor_with_key_fallback(dst: &mut [u8], key: &[u8]) {
    for (d, k) in dst.iter_mut().zip(key.iter()) {
        *d ^= *k;
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse2")]
unsafe fn xor_with_key_sse2(dst: &mut [u8], key: &[u8]) {
    use std::arch::x86_64::*;

    let len = dst.len();
    let mut i = 0;
    while i + 16 <= len {
        unsafe {
            let dst_ptr = dst.as_mut_ptr().add(i) as *mut __m128i;
            let key_ptr = key.as_ptr().add(i) as *const __m128i;
            let data = _mm_loadu_si128(dst_ptr);
            let mask = _mm_loadu_si128(key_ptr);
            let x = _mm_xor_si128(data, mask);
            _mm_storeu_si128(dst_ptr, x);
        }
        i += 16;
    }
    if i < len {
        xor_with_key_fallback(&mut dst[i..], &key[i..]);
    }
}
