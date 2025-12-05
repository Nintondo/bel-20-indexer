use nint_blk::{Coin, CoinType};

use super::{proto::*, runtime_state::BlockTokenState, structs::*, *};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum HistoryTokenAction {
    Deploy {
        tick: OriginalTokenTick,
        max: Fixed128,
        lim: Fixed128,
        dec: u8,
        recipient: FullHash,
        txid: Txid,
        vout: u32,
    },
    Mint {
        tick: OriginalTokenTick,
        amt: Fixed128,
        recipient: FullHash,
        txid: Txid,
        vout: u32,
    },
    DeployTransfer {
        tick: OriginalTokenTick,
        amt: Fixed128,
        recipient: FullHash,
        txid: Txid,
        vout: u32,
    },
    Send {
        tick: OriginalTokenTick,
        amt: Fixed128,
        recipient: FullHash,
        sender: FullHash,
        txid: Txid,
        vout: u32,
    },
}

impl HistoryTokenAction {
    pub fn tick(&self) -> OriginalTokenTick {
        match self {
            HistoryTokenAction::Deploy { tick, .. }
            | HistoryTokenAction::Mint { tick, .. }
            | HistoryTokenAction::DeployTransfer { tick, .. }
            | HistoryTokenAction::Send { tick, .. } => *tick,
        }
    }

    pub fn recipient(&self) -> FullHash {
        match self {
            HistoryTokenAction::Mint { recipient, .. } => *recipient,
            HistoryTokenAction::DeployTransfer { recipient, .. } => *recipient,
            HistoryTokenAction::Send { recipient, .. } => *recipient,
            HistoryTokenAction::Deploy { recipient, .. } => *recipient,
        }
    }

    pub fn sender(&self) -> Option<FullHash> {
        match self {
            HistoryTokenAction::Send { sender, .. } => Some(*sender),
            _ => None,
        }
    }
}

impl<'a> BlockTokenState<'a> {
    pub(crate) fn try_parse(content_type: &str, content: &[u8], height: u32, coin: CoinType) -> Result<Brc4, Brc4ParseErr> {
        // Dogecoin wonky bugfix
        if coin.name == nint_blk::Dogecoin::NAME {
            if !content_type.starts_with("text/plain") && !content_type.starts_with("application/json") {
                return Err(Brc4ParseErr::WrongContentType);
            }
        } else {
            let Some("text/plain" | "application/json") = content_type.split(';').nth(0) else {
                return Err(Brc4ParseErr::WrongContentType);
            };
        }

        // Validate UTF-8 first to preserve existing error semantics.
        let data = core::str::from_utf8(content).map_err(|_| Brc4ParseErr::InvalidUtf8)?;

        // Cheap byte-level prefilters to skip obviously irrelevant payloads without full JSON work.
        if content.len() < 4 || !content.windows(3).any(|w| w == b"\"p\"") {
            return Err(Brc4ParseErr::WrongProtocol);
        }
        if !content.windows(coin.brc_name.len()).any(|w| w.eq_ignore_ascii_case(coin.brc_name.as_bytes())) {
            return Err(Brc4ParseErr::WrongProtocol);
        }

        #[derive(Serialize, Deserialize)]
        struct Brc4Envelope {
            #[serde(rename = "p")]
            protocol: String,
            #[serde(flatten)]
            inner: Brc4,
        }

        let envelope = serde_json::from_str::<Brc4Envelope>(data).map_err(|error| match error.to_string().as_str() {
            "Invalid decimal: empty" => Brc4ParseErr::DecimalEmpty,
            "Invalid decimal: overflow from too many digits" => Brc4ParseErr::DecimalOverflow,
            "value cannot start from + or -" => Brc4ParseErr::DecimalPlusMinus,
            "value cannot start or end with ." => Brc4ParseErr::DecimalDotStartEnd,
            "value cannot contain spaces" => Brc4ParseErr::DecimalSpaces,
            "invalid digit found in string" => Brc4ParseErr::InvalidDigit,
            _ => Brc4ParseErr::WrongProtocol,
        })?;

        if envelope.protocol != coin.brc_name {
            return Err(Brc4ParseErr::WrongProtocol);
        }

        let mut brc4 = envelope.inner;

        match &mut brc4 {
            Brc4::Mint { proto } if !proto.amt.is_zero() => Ok(brc4),
            Brc4::Transfer { proto } if !proto.amt.is_zero() => Ok(brc4),
            Brc4::Deploy { proto } => {
                let err = Err(Brc4ParseErr::WrongProtocol);

                if proto.dec > DeployProto::MAX_DEC {
                    return err;
                }

                if proto.tick.len() == 5 {
                    if !proto.self_mint {
                        return err;
                    }
                    if (height as usize) < coin.self_mint_activation_height.unwrap_or_default() {
                        return err;
                    }
                };

                let lim = proto.lim.unwrap_or(proto.max);

                if proto.max.is_zero() || lim.is_zero() {
                    if proto.tick.len() == 4 {
                        return err;
                    }
                    if !proto.self_mint {
                        return err;
                    }
                }

                // Normalize unlimited self_mint tokens: when max==0, set an effective large cap for max/lim.
                if proto.max.is_zero() {
                    proto.max = Fixed128::from(u64::MAX);
                }
                if lim.is_zero() {
                    proto.lim = Some(proto.max);
                }

                Ok(brc4)
            }
            _ => Err(Brc4ParseErr::WrongProtocol),
        }
    }

    /// Parses token action from the InscriptionTemplate and records it
    /// into the per-block state.
    pub fn parse_token_action(&mut self, inc: &InscriptionTemplate, height: u32, created: u32) -> Option<TransferProto> {
        // skip to not add invalid token creation in token state
        if inc.owner.is_op_return_hash() || inc.leaked {
            return None;
        }

        let coin = self.server.indexer.coin;

        // ord/OPI-style gating for BRC20 inscriptions on p2tr-only coins.
        // Make behaviour coin-specific so that:
        //  - BTC (brc-20) keeps existing logic: reject cursed or unbound inscriptions.
        if coin.only_p2tr && (inc.cursed_for_brc20 || inc.unbound) {
            return None;
        }

        let brc4 = match Self::try_parse(inc.content_type.as_ref()?, inc.content.as_ref()?, height, coin) {
            Ok(ok) => ok,
            Err(_) => {
                return None;
            }
        };

        match brc4 {
            Brc4::Deploy { proto } => self.token_actions.push(TokenAction::Deploy {
                genesis: inc.genesis,
                proto: DeployProtoDB {
                    tick: proto.tick,
                    max: proto.max,
                    lim: proto.lim.unwrap_or(proto.max),
                    dec: proto.dec,
                    self_mint: proto.self_mint,
                    supply: Fixed128::ZERO,
                    transfer_count: 0,
                    mint_count: 0,
                    height,
                    created,
                    deployer: inc.owner,
                    transactions: 1,
                },
                owner: inc.owner,
            }),
            Brc4::Mint { proto } => {
                // if log_this_tx {
                //     eprintln!(
                //         "[DEBUG_TX] token-mint tx={} tick={:?} amt={:?} owner_opret={} cursed={} unbound={}",
                //         inc.genesis.txid,
                //         proto.tick,
                //         proto.amt,
                //         inc.owner.is_op_return_hash(),
                //         inc.cursed_for_brc20,
                //         inc.unbound
                //     );
                // }
                self.token_actions.push(TokenAction::Mint {
                    owner: inc.owner,
                    proto,
                    txid: inc.location.outpoint.txid,
                    vout: inc.location.outpoint.vout,
                });
            }
            Brc4::Transfer { proto } => {
                // if log_this_tx {
                //     eprintln!(
                //         "[DEBUG_TX] token-transfer tx={} tick={:?} amt={:?}",
                //         inc.genesis.txid,
                //         proto.tick,
                //         proto.amt
                //     );
                // }
                self.token_actions.push(TokenAction::Transfer {
                    location: inc.location,
                    owner: inc.owner,
                    proto,
                    txid: inc.location.outpoint.txid,
                    vout: inc.location.outpoint.vout,
                });
                self.all_transfers.insert(inc.location, TransferProtoDB::from_proto(proto, height).ok()?);
                return Some(proto);
            }
        };

        None
    }

    pub fn transferred(&mut self, transfer_location: Location, recipient: FullHash, txid: Txid, vout: u32) {
        self.token_actions.push(TokenAction::Transferred {
            transfer_location,
            recipient,
            txid,
            vout,
        });
    }

    pub fn burned_transfer(&mut self, location: Location, txid: Txid, vout: u32) {
        self.token_actions.push(TokenAction::Transferred {
            transfer_location: location,
            recipient: *OP_RETURN_HASH,
            txid,
            vout,
        });
    }
}
