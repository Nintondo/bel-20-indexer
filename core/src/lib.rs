use crate::types::full_hash::ComputeScriptHash;
use crate::types::full_hash::FullHash;
use lazy_static::lazy_static;

pub mod db;
pub mod types;
pub mod utils;
pub mod ports;

pub type Fixed128 = nintypes::utils::fixed::Fixed128<18>;

pub const MAINNET_START_HEIGHT: u32 = 26_371;

pub const OP_RETURN_ADDRESS: &str = "BURNED";
pub const NON_STANDARD_ADDRESS: &str = "non-standard";

lazy_static! {
    pub static ref OP_RETURN_HASH: FullHash = OP_RETURN_ADDRESS.compute_script_hash();
}

pub trait IsOpReturnHash {
    fn is_op_return_hash(&self) -> bool;
}

impl IsOpReturnHash for FullHash {
    fn is_op_return_hash(&self) -> bool {
        self.eq(&*OP_RETURN_HASH)
    }
}
