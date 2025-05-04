use super::*;

use inscriptions::structs::Partials;

generate_db_code! {
    token_to_meta: LowerCaseTokenTick => UsingSerde<TokenMetaDB>,
    address_location_to_transfer: AddressLocation => UsingSerde<TransferProtoDB>,
    address_token_to_balance: AddressToken => UsingSerde<TokenBalance>,
    address_token_to_history: AddressTokenId => UsingSerde<HistoryValue>,
    block_info: u32 => BlockInfo,
    prevouts: UsingConsensus<OutPoint> => UsingConsensus<TxOut>,
    outpoint_to_partials: UsingConsensus<OutPoint> => Partials,
    last_block: () => u32,
    last_history_id: () => u64,
    proof_of_history: u32 => UsingConsensus<sha256::Hash>,
    block_events: u32 => Vec<AddressTokenId>,
    fullhash_to_address: FullHash => String,
    outpoint_to_event: UsingConsensus<OutPoint> => AddressTokenId,
}

impl DB {
    pub fn load_token_accounts(
        &self,
        keys: Vec<AddressToken>,
    ) -> HashMap<AddressToken, TokenBalance> {
        self.address_token_to_balance
            .multi_get(keys.iter().collect_vec())
            .into_iter()
            .zip(keys)
            .flat_map(|(v, k)| v.map(|v| (k, v)))
            .collect()
    }

    pub fn load_transfers(
        &self,
        keys: BTreeSet<AddressLocation>,
    ) -> Vec<(Location, (FullHash, TransferProtoDB))> {
        let result = self
            .address_location_to_transfer
            .iter()
            .filter_map(|(k, v)| {
                keys.contains(&AddressLocation {
                    address: k.address,
                    location: Location {
                        offset: 0,
                        outpoint: k.location.outpoint,
                    },
                })
                .then_some((k.location, (k.address, v)))
            })
            .collect_vec();

        self.address_location_to_transfer
            .remove_batch(
                result
                    .iter()
                    .map(|(location, (address, _))| AddressLocation {
                        address: *address,
                        location: *location,
                    }),
            );

        result
    }
}
