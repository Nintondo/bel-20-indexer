use nintondo_dogecoin::BlockHash;
use super::*;
use crate::types::protocol::TransferProtoDB;
use crate::types::structs::{
    AddressLocation, AddressToken, AddressTokenId, HistoryValue, LowerCaseTokenTick, TokenBalance,
    TokenMetaDB,
};
use nintondo_dogecoin::hashes::sha256;
use nintondo_dogecoin::{OutPoint, TxOut};

generate_db_code! {
    token_to_meta: LowerCaseTokenTick => UsingSerde<TokenMetaDB>,
    address_location_to_transfer: AddressLocation => UsingSerde<TransferProtoDB>,
    address_token_to_balance: AddressToken => UsingSerde<TokenBalance>,
    address_token_to_history: AddressTokenId => UsingSerde<HistoryValue>,
    block_hashes: u32 => UsingConsensus<BlockHash>,
    prevouts: UsingConsensus<OutPoint> => UsingConsensus<TxOut>,
    last_block: () => u32,
    last_history_id: () => u64,
    proof_of_history: u32 => UsingConsensus<sha256::Hash>,
    block_events: u32 => Vec<AddressTokenId>,
    fullhash_to_address: FullHash => String,
    outpoint_to_event: UsingConsensus<OutPoint> => AddressTokenId,
}
