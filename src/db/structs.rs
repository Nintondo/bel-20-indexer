use bellscoin::consensus;

use super::*;
use inscriptions::structs::Part;

#[derive(Clone, Eq, PartialEq, Hash, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LowerCaseTokenTick(pub Vec<u8>);

impl<T: AsRef<[u8]>> From<T> for LowerCaseTokenTick {
    fn from(value: T) -> Self {
        LowerCaseTokenTick(
            String::from_utf8_lossy(value.as_ref())
                .to_lowercase()
                .as_bytes()
                .to_vec(),
        )
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

impl rocksdb_wrapper::Pebble for LowerCaseTokenTick {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
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
            location: Location {
                outpoint,
                offset: 0,
            },
        };
        let end = Self {
            address,
            location: Location {
                outpoint,
                offset: u64::MAX,
            },
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
                outpoint: OutPoint {
                    txid: Txid::all_zeros(),
                    vout: 0,
                },
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
            location: Location {
                outpoint: offset,
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
}

impl rocksdb_wrapper::Pebble for AddressLocation {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
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

#[derive(Clone, Debug)]
pub struct Partials {
    pub inscription_index: u32,
    pub genesis_txid: Txid,
    pub parts: Vec<Part>,
}

impl rocksdb_wrapper::Pebble for Partials {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut buffer = vec![];
        buffer.extend(v.inscription_index.to_be_bytes().to_vec());
        buffer.extend_from_slice(&bellscoin::consensus::serialize(&v.genesis_txid));

        for part in &v.parts {
            buffer.extend([part.is_tapscript as u8]);
            let script_len = part.script_buffer.len() as u32;
            buffer.extend(script_len.to_be_bytes().to_vec());
            buffer.extend(part.script_buffer.clone());
        }

        Cow::Owned(buffer)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let inscription_index = u32::from_be_bytes(v[..4].try_into()?);
        let genesis_txid: Txid = bellscoin::consensus::deserialize(&v[4..36])?;
        let mut parts = vec![];
        let mut offset = 4 + 32;
        while offset != v.len() {
            let is_tapscript = v[offset] == 1;
            offset += 1;
            let script_len = u32::from_be_bytes(v[offset..offset + 4].try_into()?) as usize;
            offset += 4;
            let script_buffer = v[offset..offset + script_len].to_vec();

            parts.push(Part {
                is_tapscript,
                script_buffer,
            });
        }

        Ok(Self {
            genesis_txid,
            inscription_index,
            parts,
        })
    }
}

#[derive(Clone, Copy)]
pub struct BlockInfo {
    pub hash: BlockHash,
    pub created: u32,
}

impl rocksdb_wrapper::Pebble for BlockInfo {
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

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct AddressTokenIdDB {
    pub address: FullHash,
    pub token: OriginalTokenTick,
    pub id: u64,
}

impl rocksdb_wrapper::Pebble for AddressTokenIdDB {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut result = Vec::with_capacity(32 + 4 + 8);
        result.extend(v.address);
        result.extend(v.token.0);
        result.extend(v.id.to_be_bytes());

        Cow::Owned(result)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let address: FullHash = v[..32].try_into().anyhow()?;
        let token = OriginalTokenTick(v[32..v.len() - 8].try_into().anyhow()?);
        let id = u64::from_be_bytes(v[v.len() - 8..].try_into().anyhow()?);

        Ok(Self { address, id, token })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Hash, Eq, PartialEq, PartialOrd, Ord)]
pub struct AddressToken {
    pub address: FullHash,
    pub token: OriginalTokenTick,
}

impl AddressToken {
    pub fn search(address: FullHash) -> RangeInclusive<AddressToken> {
        let start = AddressToken {
            address,
            token: [0; 4].into(),
        };
        let end = AddressToken {
            address,
            token: [u8::MAX; 4].into(),
        };

        start..=end
    }
}

impl From<AddressTokenIdDB> for AddressToken {
    fn from(value: AddressTokenIdDB) -> Self {
        Self {
            address: value.address,
            token: value.token,
        }
    }
}

impl rocksdb_wrapper::Pebble for AddressToken {
    type Inner = Self;

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(Self {
            address: v[..32].try_into().anyhow()?,
            token: OriginalTokenTick(v[32..].try_into().expect("Expected [u8;4], but got more")),
        })
    }

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut result = Vec::with_capacity(32 + 4);
        result.extend(v.address);
        result.extend(v.token.0);
        Cow::Owned(result)
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TransferProtoDB {
    pub tick: OriginalTokenTick,
    pub amt: Fixed128,
    pub height: u32,
}

impl TransferProtoDB {
    pub fn from_proto(value: TransferProto, height: u32) -> anyhow::Result<Self> {
        let v = value.value()?;
        Ok(Self {
            amt: v.amt,
            height,
            tick: v.tick,
        })
    }
}

impl From<TransferProtoDB> for TransferProto {
    fn from(v: TransferProtoDB) -> Self {
        if *BLOCKCHAIN == "bells" {
            TransferProto::Bel20(MintProtoWrapper {
                tick: v.tick,
                amt: v.amt,
            })
        } else {
            TransferProto::Drc20(MintProtoWrapper {
                tick: v.tick,
                amt: v.amt,
            })
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct DeployProtoDB {
    pub tick: OriginalTokenTick,
    pub max: Fixed128,
    pub lim: Fixed128,
    pub dec: u8,
    pub supply: Fixed128,
    pub transfer_count: u64,
    pub mint_count: u64,
    pub height: u32,
    pub created: u32,
    pub deployer: FullHash,
    pub transactions: u32,
}

impl DeployProtoDB {
    pub fn is_completed(&self) -> bool {
        self.supply == Fixed128::from(self.max)
    }
    pub fn mint_percent(&self) -> Fixed128 {
        (rust_decimal::Decimal::from_u64(100).unwrap() * self.supply.into_decimal()
            / self.max.into_decimal())
        .into()
    }
}

#[derive(Debug, Serialize, Deserialize, PartialOrd, Ord, PartialEq, Eq, Clone, Default)]
pub struct TokenBalance {
    pub balance: Fixed128,
    pub transferable_balance: Fixed128,
    pub transfers_count: u64,
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum TokenHistoryDB {
    Deploy {
        max: Fixed128,
        lim: Fixed128,
        dec: u8,
        txid: Txid,
        vout: u32,
    },
    Mint {
        amt: Fixed128,
        txid: Txid,
        vout: u32,
    },
    DeployTransfer {
        amt: Fixed128,
        txid: Txid,
        vout: u32,
    },
    Send {
        amt: Fixed128,
        recipient: FullHash,
        txid: Txid,
        vout: u32,
    },
    Receive {
        amt: Fixed128,
        sender: FullHash,
        txid: Txid,
        vout: u32,
    },
    SendReceive {
        amt: Fixed128,
        txid: Txid,
        vout: u32,
    },
}

#[derive(Serialize, Debug, Clone, Deserialize)]
pub struct HistoryValue {
    pub height: u32,
    pub action: TokenHistoryDB,
}

impl TokenHistoryDB {
    pub fn from_token_history(token_history: HistoryTokenAction) -> Self {
        match token_history {
            HistoryTokenAction::Deploy {
                max,
                lim,
                dec,
                txid,
                vout,
                ..
            } => TokenHistoryDB::Deploy {
                max,
                lim,
                dec,
                txid,
                vout,
            },
            HistoryTokenAction::Mint {
                amt, txid, vout, ..
            } => TokenHistoryDB::Mint { amt, txid, vout },
            HistoryTokenAction::DeployTransfer {
                amt, txid, vout, ..
            } => TokenHistoryDB::DeployTransfer { amt, txid, vout },
            HistoryTokenAction::Send {
                amt,
                recipient,
                sender,
                txid,
                vout,
                ..
            } => {
                if sender == recipient {
                    TokenHistoryDB::SendReceive { amt, txid, vout }
                } else {
                    TokenHistoryDB::Send {
                        amt,
                        recipient,
                        txid,
                        vout,
                    }
                }
            }
        }
    }

    pub fn address(&self) -> Option<&FullHash> {
        match self {
            TokenHistoryDB::Receive { sender, .. } => Some(sender),
            TokenHistoryDB::Send { recipient, .. } => Some(recipient),
            _ => None,
        }
    }

    pub fn outpoint(&self) -> OutPoint {
        match self {
            TokenHistoryDB::Deploy { txid, vout, .. }
            | TokenHistoryDB::Mint { txid, vout, .. }
            | TokenHistoryDB::DeployTransfer { txid, vout, .. }
            | TokenHistoryDB::Send { txid, vout, .. }
            | TokenHistoryDB::Receive { txid, vout, .. }
            | TokenHistoryDB::SendReceive { txid, vout, .. } => OutPoint {
                txid: *txid,
                vout: *vout,
            },
        }
    }
}
