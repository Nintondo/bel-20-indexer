use super::*;

use blockchain::proto::{
    Hashed, MerkleBranch,
    header::BlockHeader,
    tx::{EvaluatedTx, RawTx},
    varuint::VarUint,
};

/// Basic block structure which holds all information
pub struct Block {
    pub size: u32,
    pub header: Hashed<BlockHeader>,
    pub aux_pow_extension: Option<AuxPowExtension>,
    pub tx_count: VarUint,
    pub txs: Vec<Hashed<EvaluatedTx>>,
}

impl Block {
    pub fn new(
        size: u32,
        header: BlockHeader,
        aux_pow_extension: Option<AuxPowExtension>,
        tx_count: VarUint,
        txs: Vec<RawTx>,
    ) -> Block {
        let txs = txs
            .into_par_iter()
            .map(|raw| Hashed::double_sha256(EvaluatedTx::from(raw)))
            .collect();
        Block {
            size,
            header: Hashed::double_sha256(header),
            aux_pow_extension,
            tx_count,
            txs,
        }
    }

    /// Computes merkle root for all containing transactions
    pub fn compute_merkle_root(&self) -> sha256d::Hash {
        let hashes = self
            .txs
            .iter()
            .map(|tx| tx.hash)
            .collect::<Vec<sha256d::Hash>>();
        utils::merkle_root(hashes)
    }

    /// Calculates merkle root and verifies it against the field in BlockHeader.
    /// panics if not valid.
    pub fn verify_merkle_root(&self) -> Result<()> {
        let merkle_root = self.compute_merkle_root();

        if merkle_root == self.header.value.merkle_root {
            Ok(())
        } else {
            let msg = format!(
                "Invalid merkle_root!\n  -> expected: {}\n  -> got: {}\n",
                &self.header.value.merkle_root, &merkle_root
            );
            anyhow::bail!("{}", msg);
        }
    }
}

impl fmt::Debug for Block {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("Block")
            .field("header", &self.header)
            .field("tx_count", &self.tx_count)
            .finish()
    }
}

/// This is used to prove work on the auxiliary blockchain,
/// see https://en.bitcoin.it/wiki/Merged_mining_specification
pub struct AuxPowExtension {
    pub coinbase_tx: RawTx,
    pub block_hash: sha256d::Hash,
    pub coinbase_branch: MerkleBranch,
    pub blockchain_branch: MerkleBranch,
    pub parent_block: BlockHeader,
}
