use super::*;
use nintondo_dogecoin::{Address, ScriptBuf};

pub trait ScriptToAddr {
    fn to_address_str(&self, network: Network) -> Option<String>;
}

impl ScriptToAddr for ScriptBuf {
    fn to_address_str(&self, network: Network) -> Option<String> {
        Address::from_script(self, network)
            .map(|s| s.to_string())
            .ok()
    }
}

impl ScriptToAddr for &ScriptBuf {
    fn to_address_str(&self, network: Network) -> Option<String> {
        Address::from_script(self, network)
            .map(|s| s.to_string())
            .ok()
    }
}

impl ScriptToAddr for String {
    fn to_address_str(&self, _network: Network) -> Option<String> {
        let unchecked = nintondo_dogecoin::Address::from_str(self).ok()?;
        if unchecked.is_valid_for_network(*NETWORK) {
            let checked = unchecked.assume_checked().to_string();
            return Some(checked);
        }
        None
    }
}
