use std::collections::{BTreeMap, HashMap};
use crate::types::full_hash::FullHash;
use crate::types::location::Location;
use crate::types::protocol::TransferProtoDB;
use crate::types::structs::{AddressToken, LowerCaseTokenTick, TokenAction, TokenBalance, TokenMeta};

#[derive(Clone, Default)]
pub struct TokenCache {
    /// All tokens. Used to check if a transfer is valid. Used like a cache, loaded from db before parsing.
    pub tokens: HashMap<LowerCaseTokenTick, TokenMeta>,

    /// All token accounts. Used to check if a transfer is valid. Used like a cache, loaded from db before parsing.
    pub token_accounts: HashMap<AddressToken, TokenBalance>,

    /// All token actions that are not validated yet but just parsed.
    pub token_actions: Vec<TokenAction>,

    /// All transfer actions. Used to check if a transfer is valid. Used like cache.
    pub all_transfers: HashMap<Location, TransferProtoDB>,
    
    /// All transfer actions that are valid. Used to write to the db.
    pub valid_transfers: BTreeMap<Location, (FullHash, TransferProtoDB)>,
}
// impl TokenCache { // todo move
//     pub fn process_token_actions(
//         &mut self,
//         reorg_cache: Option<Arc<parking_lot::Mutex<crate::core_utils::reorg::ReorgCache>>>,
//         holders: &Holders,
//     ) -> Vec<HistoryTokenAction> {
//         let mut history = vec![];
//
//         for action in self.token_actions.drain(..) {
//             match action {
//                 TokenAction::Deploy {
//                     genesis,
//                     proto,
//                     owner,
//                 } => {
//                     let DeployProtoDB {
//                         tick,
//                         max,
//                         lim,
//                         dec,
//                         ..
//                     } = proto.clone();
//                     if let std::collections::hash_map::Entry::Vacant(e) =
//                         self.tokens.entry(tick.into())
//                     {
//                         e.insert(TokenMeta { genesis, proto });
//
//                         history.push(HistoryTokenAction::Deploy {
//                             tick,
//                             max,
//                             lim,
//                             dec,
//                             recipient: owner,
//                             txid: genesis.txid,
//                             vout: genesis.index,
//                         });
//
//                         if let Some(x) = reorg_cache.as_ref() {
//                             x.lock().added_deployed_token(tick);
//                         }
//                     }
//                 }
//                 TokenAction::Mint {
//                     owner,
//                     proto,
//                     txid,
//                     vout,
//                 } => {
//                     let MintProto::Bel20 { tick, amt } = proto;
//                     let Some(token) = self.tokens.get_mut(&tick.into()) else {
//                         continue;
//                     };
//                     let DeployProtoDB {
//                         max,
//                         lim,
//                         dec,
//                         supply,
//                         mint_count,
//                         transactions,
//                         tick,
//                         ..
//                     } = &mut token.proto;
//
//                     if amt.scale() > *dec {
//                         continue;
//                     }
//
//                     if Fixed128::from(*lim) < amt {
//                         continue;
//                     }
//
//                     if *supply == Fixed128::from(*max) {
//                         continue;
//                     }
//                     let amt = amt.min(Fixed128::from(*max) - *supply);
//                     *supply += amt;
//                     *transactions += 1;
//
//                     let key = AddressToken {
//                         address: owner,
//                         token: *tick,
//                     };
//
//                     holders.increase(
//                         &key,
//                         self.token_accounts
//                             .get(&key)
//                             .unwrap_or(&TokenBalance::default()),
//                         amt,
//                     );
//                     self.token_accounts.entry(key.clone()).or_default().balance += amt;
//                     *mint_count += 1;
//
//                     history.push(HistoryTokenAction::Mint {
//                         tick: *tick,
//                         amt,
//                         recipient: key.address,
//                         txid,
//                         vout,
//                     });
//
//                     if let Some(x) = reorg_cache.as_ref() {
//                         x.lock().added_minted_token(key, amt);
//                     }
//                 }
//                 TokenAction::Transfer {
//                     owner,
//                     location,
//                     proto,
//                     txid,
//                     vout,
//                 } => {
//                     let Some(data) = self.all_transfers.remove(&location) else {
//                         // skip cause is it transfer already spent
//                         continue;
//                     };
//
//                     let TransferProto::Bel20 { tick, amt } = proto;
//
//                     let Some(token) = self.tokens.get_mut(&tick.into()) else {
//                         continue;
//                     };
//                     let DeployProtoDB {
//                         transfer_count,
//                         dec,
//                         transactions,
//                         tick,
//                         ..
//                     } = &mut token.proto;
//
//                     if amt.scale() > *dec {
//                         // skip wrong protocol
//                         continue;
//                     }
//
//                     let key = AddressToken {
//                         address: owner,
//                         token: *tick,
//                     };
//                     let Some(account) = self.token_accounts.get_mut(&key) else {
//                         continue;
//                     };
//
//                     if amt > account.balance {
//                         continue;
//                     }
//
//                     if let Some(x) = reorg_cache.as_ref() {
//                         x.lock().added_transfer_token(location, key.clone(), amt);
//                     }
//
//                     account.balance -= amt;
//                     account.transfers_count += 1;
//                     account.transferable_balance += amt;
//
//                     history.push(HistoryTokenAction::DeployTransfer {
//                         tick: *tick,
//                         amt,
//                         recipient: key.address,
//                         txid,
//                         vout,
//                     });
//
//                     self.valid_transfers.insert(location, (key.address, data));
//                     *transfer_count += 1;
//                     *transactions += 1;
//                 }
//                 TokenAction::Transferred {
//                     transfer_location,
//                     recipient,
//                     txid,
//                     vout,
//                 } => {
//                     let Some((sender, TransferProtoDB { tick, amt, height })) =
//                         self.valid_transfers.remove(&transfer_location)
//                     else {
//                         // skip cause transfer has been already spent
//                         continue;
//                     };
//
//                     let token = self.tokens.get_mut(&tick.into()).expect("Tick must exist");
//
//                     let DeployProtoDB {
//                         transactions, tick, ..
//                     } = &mut token.proto;
//
//                     let old_key = AddressToken {
//                         address: sender,
//                         token: *tick,
//                     };
//
//                     let old_account = self.token_accounts.get_mut(&old_key).unwrap();
//                     if old_account.transfers_count == 0 || old_account.transferable_balance < amt {
//                         panic!("Invalid transfer sender balance");
//                     }
//
//                     holders.decrease(&old_key, old_account, amt);
//                     old_account.transfers_count -= 1;
//                     old_account.transferable_balance -= amt;
//                     *transactions += 1;
//
//                     if !recipient.is_op_return_hash() {
//                         let recipient_key = AddressToken {
//                             address: recipient,
//                             token: *tick,
//                         };
//
//                         holders.increase(
//                             &recipient_key,
//                             self.token_accounts
//                                 .get(&recipient_key)
//                                 .unwrap_or(&TokenBalance::default()),
//                             amt,
//                         );
//
//                         self.token_accounts
//                             .entry(recipient_key)
//                             .or_default()
//                             .balance += amt;
//                     }
//
//                     history.push(HistoryTokenAction::Send {
//                         amt,
//                         tick: *tick,
//                         recipient,
//                         sender,
//                         txid,
//                         vout,
//                     });
//
//                     if let Some(x) = reorg_cache.as_ref() {
//                         x.lock().removed_transfer_token(
//                             AddressLocation {
//                                 address: sender,
//                                 location: transfer_location,
//                             },
//                             TransferProtoDB {
//                                 tick: *tick,
//                                 amt,
//                                 height,
//                             },
//                             recipient,
//                         );
//                     }
//                 }
//             }
//         }
//
//         history
//     }
// }
