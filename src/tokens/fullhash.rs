use super::*;
use crate::inscriptions::types::ParsedTokenAddress;
use std::ops::Deref;

#[derive(Serialize, Deserialize, Clone, Debug, Hash, Eq, PartialEq, PartialOrd, Ord, Copy)]
#[repr(transparent)]
#[serde(transparent)]
pub struct FullHash([u8; 32]);

impl FullHash {
    pub const ZERO: Self = Self([0; 32]);
}

impl_pebble!(FullHash = [u8; 32]);

impl Deref for FullHash {
    type Target = [u8; 32];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<[u8; 32]> for FullHash {
    fn from(value: [u8; 32]) -> Self {
        Self(value)
    }
}

impl IntoIterator for FullHash {
    type Item = u8;
    type IntoIter = std::array::IntoIter<u8, 32>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl TryFrom<&[u8]> for FullHash {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let result: [u8; 32] = value
            .try_into()
            .map_err(|_| anyhow::anyhow!("Failed to convert &[u8] to FullHash"))?;

        Ok(Self(result))
    }
}

impl TryFrom<Vec<u8>> for FullHash {
    type Error = anyhow::Error;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let result: [u8; 32] = value
            .try_into()
            .map_err(|_| anyhow::anyhow!("Failed to convert Vec<u8> to FullHash"))?;

        Ok(Self(result))
    }
}

fn compute_script_hash(data: impl AsRef<[u8]>) -> FullHash {
    let mut hasher = <sha2::Sha256 as sha2::digest::Digest>::new();
    sha2::digest::Update::update(&mut hasher, data.as_ref());
    let bytes: [u8; 32] = sha2::digest::Digest::finalize(hasher)[..]
        .try_into()
        .expect("SHA256 size is 32 bytes");
    bytes.into()
}

pub trait ComputeScriptHash {
    fn compute_script_hash(&self) -> FullHash;
}

impl ComputeScriptHash for script::Script {
    fn compute_script_hash(&self) -> FullHash {
        compute_script_hash(self.as_bytes())
    }
}

impl ComputeScriptHash for &'static str {
    fn compute_script_hash(&self) -> FullHash {
        compute_script_hash(self.as_bytes())
    }
}

impl ComputeScriptHash for String {
    fn compute_script_hash(&self) -> FullHash {
        compute_script_hash(self.as_bytes())
    }
}

impl ComputeScriptHash for ParsedTokenAddress {
    fn compute_script_hash(&self) -> FullHash {
        match self {
            ParsedTokenAddress::Standard(str) if !str.is_op_return() => str.compute_script_hash(),
            ParsedTokenAddress::Standard(_) => *OP_RETURN_HASH,
            ParsedTokenAddress::NonStandard(hash) => *hash,
        }
    }
}
