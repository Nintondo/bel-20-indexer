use std::borrow::Cow;
use bellscoin::BlockHash;
use bellscoin::hashes::Hash;
use crate::db::Pebble;

pub struct BlockInfo {
    pub hash: BlockHash,
    pub created: u32,
}

impl Pebble for BlockInfo {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        Cow::Owned(
            [
                v.hash.to_byte_array().as_slice(),
                v.created.to_be_bytes().as_slice(),
            ]
                .concat(),
        )
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let hash = BlockHash::from_byte_array(v[0..32].try_into()?);
        let created = u32::from_be_bytes(v[32..].try_into()?);

        Ok(Self { created, hash })
    }
}
