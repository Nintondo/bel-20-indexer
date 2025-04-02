use nintondo_dogecoin::{Address, PublicKey};
use script::ScriptBuf;
use validator::ValidationError;

use super::*;

pub fn to_scripthash(
    script_type: &str,
    script_str: &str,
    network: Network,
) -> anyhow::Result<FullHash> {
    let Ok(pubkey) = PublicKey::from_str(script_str) else {
        return match script_type {
            "address" => address_to_scripthash(script_str, network),
            "scripthash" => parse_scripthash(script_str),
            _ => anyhow::bail!("Invalid script type"),
        };
    };
    Ok(ScriptBuf::new_p2pk(&pubkey).compute_script_hash())
}

fn address_to_scripthash(addr: &str, network: Network) -> anyhow::Result<FullHash> {
    let addr = Address::from_str(addr)?;

    let is_expected_net = {
        // Testnet, Regtest and Signet all share the same version bytes,
        // `addr_network` will be detected as Testnet for all of them.
        addr.network == network
    };

    if !is_expected_net {
        anyhow::bail!("Address on invalid network");
    }

    Ok(addr.payload.script_pubkey().compute_script_hash())
}

fn parse_scripthash(scripthash: &str) -> anyhow::Result<FullHash> {
    let bytes = hex::decode(scripthash)?;
    bytes.try_into()
}

pub fn page_size_default() -> usize {
    6
}

pub fn first_page() -> usize {
    1
}

pub fn validate_tick(tick: &str) -> Result<(), ValidationError> {
    if tick.len() != 4 {
        return Err(ValidationError::new("Wrong tick length"));
    }

    Ok(())
}
