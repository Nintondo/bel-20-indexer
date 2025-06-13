use super::*;

use inscriptions::structs::Partials;

rocksdb_wrapper::generate_db_code! {
    token_to_meta: LowerCaseTokenTick => UsingSerde<TokenMetaDB>,
    address_location_to_transfer: AddressLocation => UsingSerde<TransferProtoDB>,
    address_token_to_balance: AddressToken => UsingSerde<TokenBalance>,
    address_token_to_history: AddressTokenId => UsingSerde<HistoryValue>,
    block_info: u32 => BlockInfo,
    prevouts: UsingConsensus<OutPoint> => UsingConsensus<TxOut>,
    outpoint_to_partials: UsingConsensus<OutPoint> => Partials,
    outpoint_to_inscription_offsets: UsingConsensus<OutPoint> => UsingSerde<HashSet<u64>>,
    last_block: () => u32,
    last_history_id: () => u64,
    proof_of_history: u32 => UsingConsensus<sha256::Hash>,
    block_events: u32 => UsingSerde<Vec<AddressTokenId>>,
    fullhash_to_address: FullHash => String,
    outpoint_to_event: UsingConsensus<OutPoint> => AddressTokenId,
}

impl DB {
    pub fn load_token_accounts(
        &self,
        keys: Vec<AddressToken>,
    ) -> HashMap<AddressToken, TokenBalance> {
        self.address_token_to_balance
            .multi_get_kv(keys.iter(), false)
            .into_iter()
            .map(|(k, v)| (k.clone(), v))
            .collect()
    }

    pub fn load_transfers(
        &self,
        keys: &HashSet<AddressOutPoint>,
    ) -> Vec<(Location, (FullHash, TransferProtoDB))> {
        keys.iter()
            .flat_map(|x| {
                let (from, to) = AddressLocation::search(x.address, Some(x.outpoint)).into_inner();
                self.address_location_to_transfer
                    .range(&from..=&to, false)
                    .collect_vec()
            })
            .map(|(key, value)| (key.location, (key.address, value)))
            .collect()
    }
}
