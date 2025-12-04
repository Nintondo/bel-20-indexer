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
    pub(crate) fn try_parse(content_type: &str, content: &[u8], coin: CoinType) -> Result<Brc4, Brc4ParseErr> {
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

        let brc4 = envelope.inner;

        match &brc4 {
            Brc4::Mint { proto } if !proto.amt.is_zero() => Ok(brc4),
            Brc4::Transfer { proto } if !proto.amt.is_zero() => Ok(brc4),
            Brc4::Deploy { proto } => {
                let dec_ok = proto.dec <= DeployProto::MAX_DEC;
                // For max=0, allow self_mint tokens regardless of lim presence/value (normalized later)
                // For max>0, require non-zero effective lim
                let ok = if proto.max.is_zero() {
                    proto.self_mint
                } else {
                    !proto.lim.unwrap_or(proto.max).is_zero()
                };
                // Enforce self_mint policy by length:
                //  - 5-byte tickers must be self_mint
                //  - 4-byte tickers must NOT be self_mint
                let tick_len_ok = if proto.tick.len() == 5 { proto.self_mint } else { !proto.self_mint };
                if dec_ok && ok && tick_len_ok {
                    Ok(brc4)
                } else {
                    Err(Brc4ParseErr::WrongProtocol)
                }
            }
            &Brc4::Mint { .. } | &Brc4::Transfer { .. } => Err(Brc4ParseErr::WrongProtocol),
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

        let brc4 = match Self::try_parse(inc.content_type.as_ref()?, inc.content.as_ref()?, coin) {
            Ok(ok) => ok,
            Err(_) => {
                return None;
            }
        };

        match brc4 {
            Brc4::Deploy { proto } => {
                let v = proto;

                // Activation and policy checks
                let act = coin.self_mint_activation_height;
                let is_5_byte = v.tick.len() == 5;
                // Policy:
                //  - 5-byte tickers: require activation AND self_mint=true
                //  - 4-byte tickers: self_mint must be false
                if is_5_byte {
                    if !act.map(|h| (height as usize) >= h).unwrap_or(false) {
                        return None;
                    }
                    if !v.self_mint {
                        return None;
                    }
                } else if v.self_mint {
                    return None;
                }

                // Reject tickers containing a null byte (reference parity and safety)
                // if v.tick.as_bytes().iter().any(|&b| b == 0) {
                //     return None;
                // }

                // Normalize unlimited self_mint tokens: when max==0, set an effective large cap for max/lim.
                let mut norm_max = v.max;
                let mut norm_lim = v.lim.unwrap_or(v.max);
                if v.self_mint && norm_max.is_zero() {
                    let cap = Fixed128::from(u64::MAX);
                    norm_max = cap;
                    if norm_lim.is_zero() {
                        norm_lim = cap;
                    }
                }

                self.token_actions.push(TokenAction::Deploy {
                    genesis: inc.genesis,
                    proto: DeployProtoDB {
                        tick: v.tick,
                        max: norm_max,
                        lim: norm_lim,
                        dec: v.dec,
                        self_mint: v.self_mint,
                        supply: Fixed128::ZERO,
                        transfer_count: 0,
                        mint_count: 0,
                        height,
                        created,
                        deployer: inc.owner,
                        transactions: 1,
                    },
                    owner: inc.owner,
                })
            }
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
