use bellscoin::{consensus, OutPoint, Txid};

use super::*;
use inscriptions::structs::Part;

#[derive(Clone, Eq, PartialEq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LowerCaseTokenTick(pub Vec<u8>);

impl<T: AsRef<[u8]>> From<T> for LowerCaseTokenTick {
    fn from(value: T) -> Self {
        LowerCaseTokenTick(String::from_utf8_lossy(value.as_ref()).to_lowercase().as_bytes().to_vec())
    }
}

impl std::ops::Deref for LowerCaseTokenTick {
    type Target = Vec<u8>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for LowerCaseTokenTick {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl LowerCaseTokenTick {
    pub fn starts_with(&self, search: &str) -> bool {
        self.0.starts_with(search.to_lowercase().as_bytes())
    }
}

impl rocksdb_wrapper::Pebble for LowerCaseTokenTick {
    type Inner = Self;

    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]> {
        Cow::Borrowed(&v.0)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(Self(v.into_owned()))
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TokenMetaDB {
    pub genesis: InscriptionId,
    pub proto: DeployProtoDB,
}

impl TokenMetaDB {
    pub fn is_completed(&self) -> bool {
        self.proto.is_completed()
    }
}

impl From<TokenMeta> for TokenMetaDB {
    fn from(meta: TokenMeta) -> Self {
        TokenMetaDB {
            genesis: meta.genesis,
            proto: meta.proto,
        }
    }
}

impl From<TokenMetaDB> for TokenMeta {
    fn from(meta: TokenMetaDB) -> Self {
        TokenMeta {
            genesis: meta.genesis,
            proto: meta.proto,
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, PartialOrd, Ord, Eq)]
pub struct AddressLocation {
    pub address: FullHash,
    pub location: Location,
}

impl AddressLocation {
    pub fn search_with_offset(address: FullHash, outpoint: OutPoint) -> RangeInclusive<Self> {
        let start = Self {
            address,
            location: Location { outpoint, offset: 0 },
        };
        let end = Self {
            address,
            location: Location { outpoint, offset: u64::MAX },
        };

        start..=end
    }

    pub fn search(address: FullHash, offset: Option<OutPoint>) -> RangeInclusive<Self> {
        if let Some(offset) = offset {
            return Self::search_offset(address, offset);
        }

        let start = Self {
            address,
            location: Location {
                outpoint: OutPoint { txid: Txid::all_zeros(), vout: 0 },
                offset: 0,
            },
        };
        let end = Self {
            address,
            location: Location {
                outpoint: OutPoint {
                    txid: Txid::from_byte_array([u8::MAX; 32]),
                    vout: u32::MAX,
                },
                offset: u64::MAX,
            },
        };

        start..=end
    }

    fn search_offset(address: FullHash, offset: OutPoint) -> RangeInclusive<Self> {
        let start = Self {
            address,
            location: Location { outpoint: offset, offset: 0 },
        };
        let end = Self {
            address,
            location: Location {
                outpoint: OutPoint {
                    txid: Txid::from_byte_array([u8::MAX; 32]),
                    vout: u32::MAX,
                },
                offset: u64::MAX,
            },
        };

        start..=end
    }
}

impl rocksdb_wrapper::Pebble for AddressLocation {
    type Inner = Self;

    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]> {
        let mut result = Vec::with_capacity(32 + 44);

        result.extend(v.address);

        result.extend(consensus::serialize(&v.location.outpoint));
        result.extend(v.location.offset.to_be_bytes());

        Cow::Owned(result)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let address = v[..32].try_into().anyhow()?;
        let outpoint: OutPoint = consensus::deserialize(&v[32..32 + 36])?;
        let offset = u64::from_be_bytes(v[32 + 32 + 4..].try_into().anyhow()?);

        Ok(Self {
            address,
            location: Location { outpoint, offset },
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Partials {
    pub inscription_index: u32,
    pub genesis_txid: Txid,
    pub parts: Vec<Part>,
}

#[derive(Clone, Copy, Debug)]
pub struct TxPrevout {
    pub script_hash: FullHash,
    pub value: u64,
}

impl From<TxOut> for TxPrevout {
    fn from(tx_out: TxOut) -> Self {
        Self {
            script_hash: tx_out.script_pubkey.compute_script_hash(),
            value: tx_out.value,
        }
    }
}

impl rocksdb_wrapper::Pebble for TxPrevout {
    type Inner = Self;

    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]> {
        let mut result = Vec::with_capacity(32 + 8);
        result.extend(v.script_hash);
        result.extend(v.value.to_be_bytes());
        Cow::Owned(result)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let script_hash: FullHash = v[..32].try_into().anyhow()?;
        let value = u64::from_be_bytes(v[32..].try_into().anyhow()?);

        Ok(Self { script_hash, value })
    }
}

impl rocksdb_wrapper::Pebble for Partials {
    type Inner = Self;

    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]> {
        let mut buff = Vec::with_capacity(4 + 32 + v.parts.len() * (1 + 4 + 1700));

        buff.extend(v.inscription_index.to_be_bytes());
        buff.extend_from_slice(&bellscoin::consensus::serialize(&v.genesis_txid));

        for part in &v.parts {
            buff.push(part.is_tapscript as u8);
            let script_len = part.script_buffer.len() as u32;
            buff.extend(script_len.to_be_bytes());
            buff.extend(part.script_buffer.clone());
        }

        Cow::Owned(buff)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let inscription_index = u32::from_be_bytes(v[..4].try_into()?);

        let genesis_txid = bellscoin::consensus::deserialize(&v[4..32 + 4])?;

        let mut parts = Vec::new();

        let mut current_byte = 32 + 4;

        while current_byte != v.len() {
            let is_tapscript = v[current_byte] == 1;
            current_byte += 1;

            let script_len = u32::from_be_bytes(v[current_byte..current_byte + 4].try_into()?) as usize;
            current_byte += 4;

            let script_buffer = v[current_byte..current_byte + script_len].to_vec();
            current_byte += script_len;

            parts.push(Part { is_tapscript, script_buffer })
        }

        Ok(Partials {
            inscription_index,
            genesis_txid,
            parts,
        })
    }
}

mod balances;
pub use balances::*;
