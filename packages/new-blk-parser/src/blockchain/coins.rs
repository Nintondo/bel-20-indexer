use super::*;

pub struct EncoderConfig {
    pubkey_address: u8,
    script_address: u8,
    bech32: &'static str,
}

/// Trait to specify the underlying coin of a blockchain
/// Needs a proper magic value and a network id for address prefixes
pub trait Coin {
    /// Human readable coin name
    const NAME: &'static str;
    /// Configuration for address generation
    const CONFIG: EncoderConfig;
}

pub struct Bitcoin;
impl Coin for Bitcoin {
    const NAME: &'static str = "Bitcoin";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 0,
        script_address: 5,
        bech32: "bc",
    };
}

pub struct BitcoinTestnet;
impl Coin for BitcoinTestnet {
    const NAME: &'static str = "Bitcoin Testnet";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 113,
        script_address: 196,
        bech32: "tb",
    };
}

pub struct Litecoin;
impl Coin for Litecoin {
    const NAME: &'static str = "Litecoin";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 0,
        script_address: 5,
        bech32: "lt",
    };
}

pub struct LitecoinTestnet;
impl Coin for LitecoinTestnet {
    const NAME: &'static str = "Litecoin Testnet";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 111,
        script_address: 196,
        bech32: "tlt",
    };
}

pub struct Dogecoin;
impl Coin for Dogecoin {
    const NAME: &'static str = "Dogecoin";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 30,
        script_address: 22,
        bech32: "dg",
    };
}

pub struct DogecoinTestnet;
impl Coin for DogecoinTestnet {
    const NAME: &'static str = "Dogecoin Testnet";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 113,
        script_address: 196,
        bech32: "tdg",
    };
}

pub struct DogecoinEv;
impl Coin for DogecoinEv {
    const NAME: &'static str = "DogecoinEV";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 30,
        script_address: 22,
        bech32: "dev",
    };
}

pub struct DogecoinEvTestnet;
impl Coin for DogecoinEvTestnet {
    const NAME: &'static str = "DogecoinEV Testnet";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 113,
        script_address: 196,
        bech32: "tdev",
    };
}

pub struct Bellscoin;
impl Coin for Bellscoin {
    const NAME: &'static str = "Bellscoin";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 25,
        script_address: 30,
        bech32: "bel",
    };
}

pub struct BellscoinTestnet;
impl Coin for BellscoinTestnet {
    const NAME: &'static str = "Bellscoin Testnet";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 33,
        script_address: 22,
        bech32: "tbel",
    };
}

pub struct Pepecoin;
impl Coin for Pepecoin {
    const NAME: &'static str = "Pepecoin";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 56,
        script_address: 22,
        bech32: "pe",
    };
}

pub struct PepecoinTestnet;
impl Coin for PepecoinTestnet {
    const NAME: &'static str = "Pepecoin Testnet";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 113,
        script_address: 196,
        bech32: "tpe",
    };
}

#[derive(Clone, Copy)]
// Holds the selected coin type information
pub struct CoinType {
    pub name: &'static str,
    pub pubkey_address: u8,
    pub script_address: u8,
    pub bech32: &'static str,
}

impl Default for CoinType {
    #[inline]
    fn default() -> Self {
        CoinType::from(Bitcoin)
    }
}

impl<T: Coin> From<T> for CoinType {
    fn from(_: T) -> Self {
        let config = T::CONFIG;
        CoinType {
            name: T::NAME,
            bech32: config.bech32,
            pubkey_address: config.pubkey_address,
            script_address: config.script_address,
        }
    }
}

impl FromStr for CoinType {
    type Err = anyhow::Error;
    fn from_str(coin_name: &str) -> Result<Self> {
        match coin_name {
            "bitcoin" => Ok(CoinType::from(Bitcoin)),
            "bitcoin-testnet" => Ok(CoinType::from(BitcoinTestnet)),
            "litecoin" => Ok(CoinType::from(Litecoin)),
            "litecoin-testnet" => Ok(CoinType::from(LitecoinTestnet)),
            "dogecoin" => Ok(CoinType::from(Dogecoin)),
            "dogecoin-testnet" => Ok(CoinType::from(DogecoinTestnet)),
            "dogecoinev" => Ok(CoinType::from(DogecoinEv)),
            "dogecoinev-testnet" => Ok(CoinType::from(DogecoinEvTestnet)),
            "bellscoin" => Ok(CoinType::from(Bellscoin)),
            "bellscoin-testnet" => Ok(CoinType::from(BellscoinTestnet)),
            "pepecoin" => Ok(CoinType::from(Pepecoin)),
            "pepecoin-testnet" => Ok(CoinType::from(PepecoinTestnet)),
            n => anyhow::bail!("There is no implementation for `{}`!", n),
        }
    }
}
