use crate::{
    Fixed128,
    types::{
        full_hash::FullHash,
        location::Location,
        protocol::TransferProtoDB,
        structs::{AddressLocation, AddressToken, AddressTokenId, OriginalTokenTick},
    },
};

pub trait ReorgCacheInterface {
    fn added_deployed_token(&mut self, tick: OriginalTokenTick);
    fn added_minted_token(&mut self, token: AddressToken, amount: Fixed128);
    fn added_history(&mut self, key: AddressTokenId);
    fn added_transfer_token(&mut self, location: Location, token: AddressToken, amount: Fixed128);
    fn removed_transfer_token(
        &mut self,
        key: AddressLocation,
        value: TransferProtoDB,
        recipient: FullHash,
    );
}
