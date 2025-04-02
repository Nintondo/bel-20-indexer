use super::*;

generate_db_code! {
    token_to_meta: LowerCaseTick => UsingSerde<TokenMetaDB>,
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
