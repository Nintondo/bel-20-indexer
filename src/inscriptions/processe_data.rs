use bellscoin::{OutPoint, TxOut};
use bellscoin::hashes::sha256;
use std::collections::{HashMap, HashSet};
use itertools::Itertools;
use crate::inscriptions::structs::Partials;
use crate::server::BlockInfo;
use crate::tables::DB;
use crate::tokens::{AddressLocation, AddressToken, AddressTokenId, FullHash, HistoryValue, LowerCaseTokenTick, TokenBalance, TokenMetaDB, TransferProtoDB};

pub trait ProcessedData: Send + Sync {
    fn write(&self, db: &DB);
}

pub struct BlockInfoWriter {
    pub block_number: u32,
    pub block_info: BlockInfo,
}

impl ProcessedData for BlockInfoWriter {
    fn write(&self, db: &DB) {
        db.last_block.set((), self.block_number);
        db.block_info
            .set(self.block_number, self.block_info.clone());
    }
}

pub struct BlockPrevoutsWriter {
    pub to_write: HashMap<OutPoint, TxOut>,
    pub to_remove: Vec<OutPoint>,
}

impl ProcessedData for BlockPrevoutsWriter {
    fn write(&self, db: &DB) {
        db.prevouts.remove_batch(self.to_remove.clone().into_iter());
        db.prevouts.extend(self.to_write.clone());
    }
}

pub struct BlockFullHashWriter {
    pub addresses: Vec<(FullHash, String)>,
}

impl ProcessedData for BlockFullHashWriter {
    fn write(&self, db: &DB) {
        db.fullhash_to_address.extend(self.addresses.clone());
    }
}

pub struct BlockProofWriter {
    pub block_number: u32,
    pub block_proof: sha256::Hash,
}

impl ProcessedData for BlockProofWriter {
    fn write(&self, db: &DB) {
        db.proof_of_history.set(self.block_number, self.block_proof);
    }
}

pub struct BlockHistoryWriter {
    pub block_number: u32,
    pub last_history_id: u64,
    pub history: Vec<(AddressTokenId, HistoryValue)>,
}

impl ProcessedData for BlockHistoryWriter {
    fn write(&self, db: &DB) {
        let block_events: Vec<_> = self
            .history
            .iter()
            .map(|(address_token_id, _)| address_token_id.clone())
            .sorted_unstable_by_key(|address_token_id| address_token_id.id)
            .collect();

        let outpoint_to_event = self
            .history
            .iter()
            .map(|(address_token_id, history_value)| {
                (history_value.action.outpoint(), address_token_id.clone())
            });

        db.block_events.set(self.block_number, block_events);
        db.outpoint_to_event.extend(outpoint_to_event);
        db.address_token_to_history.extend(self.history.clone());
        db.last_history_id.set((), self.last_history_id);
    }
}

pub struct BlockTokensWriter {
    pub metas: Vec<(LowerCaseTokenTick, TokenMetaDB)>,
    pub balances: Vec<(AddressToken, TokenBalance)>,
    pub transfers_to_write: Vec<(AddressLocation, TransferProtoDB)>,
    pub transfers_to_remove: Vec<AddressLocation>,
}

impl ProcessedData for BlockTokensWriter {
    fn write(&self, db: &DB) {
        db.token_to_meta.extend(self.metas.clone());
        db.address_token_to_balance.extend(self.balances.clone());
        db.address_location_to_transfer
            .remove_batch(self.transfers_to_remove.clone().into_iter());
        db.address_location_to_transfer
            .extend(self.transfers_to_write.clone());
    }
}

pub struct BlockInscriptionPartialsWriter {
    pub to_remove: Vec<OutPoint>,
    pub to_write: Vec<(OutPoint, Partials)>,
}

impl ProcessedData for BlockInscriptionPartialsWriter {
    fn write(&self, db: &DB) {
        db.outpoint_to_partials
            .remove_batch(self.to_remove.clone().into_iter());
        db.outpoint_to_partials.extend(self.to_write.clone());
    }
}

pub struct BlockInscriptionOffsetWriter {
    pub to_remove: Vec<OutPoint>,
    pub to_write: Vec<(OutPoint, HashSet<u64>)>,
}

impl ProcessedData for BlockInscriptionOffsetWriter {
    fn write(&self, db: &DB) {
        db.outpoint_to_inscription_offsets
            .remove_batch(self.to_remove.clone().into_iter());
        db.outpoint_to_inscription_offsets
            .extend(self.to_write.clone());
    }
}