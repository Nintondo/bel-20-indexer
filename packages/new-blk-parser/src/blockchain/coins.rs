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
    /// First inscription block
    const FIB: Option<u32>;
    /// Jubilee height
    const JUBILEE_HEIGHT: Option<usize>;
    /// BRC-20 protocol name
    const BRC_NAME: &'static str;
    const ONLY_P2RT: bool;
}

pub struct Bitcoin;
impl Coin for Bitcoin {
    const NAME: &'static str = "Bitcoin";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 0,
        script_address: 5,
        bech32: "bc",
    };
    const FIB: Option<u32> = Some(767_430);
    const JUBILEE_HEIGHT: Option<usize> = Some(824_544);
    const BRC_NAME: &'static str = "brc-20";
    const ONLY_P2RT: bool = true;
}

pub struct BitcoinTestnet;
impl Coin for BitcoinTestnet {
    const NAME: &'static str = "Bitcoin Testnet";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 111,
        script_address: 196,
        bech32: "tb",
    };
    const FIB: Option<u32> = Some(2_413_343);
    const JUBILEE_HEIGHT: Option<usize> = Some(2_544_192);
    const BRC_NAME: &'static str = "brc-20";
    const ONLY_P2RT: bool = true;
}

pub struct Litecoin;
impl Coin for Litecoin {
    const NAME: &'static str = "Litecoin";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 48,
        script_address: 50,
        bech32: "ltc",
    };
    const FIB: Option<u32> = Some(2_424_429);
    const JUBILEE_HEIGHT: Option<usize> = Some(2_608_704);
    const BRC_NAME: &'static str = "ltc-20";
    const ONLY_P2RT: bool = true;
}

pub struct LitecoinTestnet;
impl Coin for LitecoinTestnet {
    const NAME: &'static str = "Litecoin Testnet";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 111,
        script_address: 58,
        bech32: "tltc",
    };
    const FIB: Option<u32> = Some(2_669_127);
    const JUBILEE_HEIGHT: Option<usize> = Some(3_096_576);
    const BRC_NAME: &'static str = "ltc-20";
    const ONLY_P2RT: bool = true;
}

pub struct Dogecoin;
impl Coin for Dogecoin {
    const NAME: &'static str = "Dogecoin";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 30,
        script_address: 22,
        bech32: "dg",
    };
    const FIB: Option<u32> = Some(4_609_001);
    const JUBILEE_HEIGHT: Option<usize> = Some(usize::MAX);
    const BRC_NAME: &'static str = "drc-20";
    const ONLY_P2RT: bool = false;
}

pub struct DogecoinTestnet;
impl Coin for DogecoinTestnet {
    const NAME: &'static str = "Dogecoin Testnet";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 113,
        script_address: 196,
        bech32: "tdg",
    };
    const FIB: Option<u32> = Some(4_260_001);
    const JUBILEE_HEIGHT: Option<usize> = Some(usize::MAX);
    const BRC_NAME: &'static str = "drc-20";
    const ONLY_P2RT: bool = false;
}

pub struct Bellscoin;
impl Coin for Bellscoin {
    const NAME: &'static str = "Bellscoin";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 25,
        script_address: 30,
        bech32: "bel",
    };
    const FIB: Option<u32> = Some(26_371);
    const JUBILEE_HEIGHT: Option<usize> = Some(133_000);
    const BRC_NAME: &'static str = "bel-20";
    const ONLY_P2RT: bool = false;
}

pub struct BellscoinTestnet;
impl Coin for BellscoinTestnet {
    const NAME: &'static str = "Bellscoin Testnet";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 33,
        script_address: 22,
        bech32: "tbel",
    };
    const FIB: Option<u32> = None;
    const JUBILEE_HEIGHT: Option<usize> = None;
    const BRC_NAME: &'static str = "bel-20";
    const ONLY_P2RT: bool = false;
}

pub struct Pepecoin;
impl Coin for Pepecoin {
    const NAME: &'static str = "Pepecoin";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 56,
        script_address: 22,
        bech32: "pe",
    };
    const FIB: Option<u32> = None;
    const JUBILEE_HEIGHT: Option<usize> = None;
    const BRC_NAME: &'static str = "prc-20";
    const ONLY_P2RT: bool = false;
}

pub struct PepecoinTestnet;
impl Coin for PepecoinTestnet {
    const NAME: &'static str = "Pepecoin Testnet";
    const CONFIG: EncoderConfig = EncoderConfig {
        pubkey_address: 113,
        script_address: 196,
        bech32: "tpe",
    };
    const FIB: Option<u32> = None;
    const JUBILEE_HEIGHT: Option<usize> = None;
    const BRC_NAME: &'static str = "prc-20";
    const ONLY_P2RT: bool = false;
}

#[derive(Clone, Copy, Debug)]
// Holds the selected coin type information
pub struct CoinType {
    pub name: &'static str,
    pub pubkey_address: u8,
    pub script_address: u8,
    pub bech32: &'static str,
    pub brc_name: &'static str,
    pub jubilee_height: Option<usize>,
    pub fib: Option<u32>,
    pub only_p2tr: bool,
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
            brc_name: T::BRC_NAME,
            jubilee_height: T::JUBILEE_HEIGHT,
            fib: T::FIB,
            only_p2tr: T::ONLY_P2RT,
        }
    }
}

impl CoinType {
    #[inline]
    pub fn has_mweb_extension_metadata(self) -> bool {
        matches!(self.name, "Litecoin" | "Litecoin Testnet")
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
            "bellscoin" => Ok(CoinType::from(Bellscoin)),
            "bellscoin-testnet" => Ok(CoinType::from(BellscoinTestnet)),
            "pepecoin" => Ok(CoinType::from(Pepecoin)),
            "pepecoin-testnet" => Ok(CoinType::from(PepecoinTestnet)),
            n => anyhow::bail!("There is no implementation for `{}`!", n),
        }
    }
}
