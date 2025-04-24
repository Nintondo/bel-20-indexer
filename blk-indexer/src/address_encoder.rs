use bellscoin::{
    address::{Payload, WitnessProgram, WitnessVersion},
    base58, bech32,
    hashes::Hash,
    PubkeyHash, ScriptHash,
};

pub enum NetworkDecoder {
    Mainnet,
    Testnet,
    Signet,
    Regtest,
}

impl From<bellscoin::Network> for NetworkDecoder {
    fn from(value: bellscoin::Network) -> Self {
        match value {
            bellscoin::Network::Bellscoin => NetworkDecoder::Mainnet,
            bellscoin::Network::Testnet => NetworkDecoder::Testnet,
            bellscoin::Network::Signet => NetworkDecoder::Signet,
            bellscoin::Network::Regtest => NetworkDecoder::Regtest,
            _ => unimplemented!(),
        }
    }
}

pub struct ConfigDecoder {
    p2pkh_prefix: u8,
    p2sh_prefix: u8,
    bech32_hrp: String,
}

pub trait EncoderParams: Send + Sync {
    fn get_pubkey_address_prefix_main(&self) -> u8;
    fn get_script_address_prefix_main(&self) -> u8;
    fn get_pubkey_address_prefix_test(&self) -> u8;
    fn get_script_address_prefix_test(&self) -> u8;
    fn get_bech32_main(&self) -> String;
    fn get_bech32_test(&self) -> String;
    fn get_bech32_reg(&self) -> String;
    fn network(&self) -> &NetworkDecoder;
}

pub trait Encoder: EncoderParams + Send + Sync {
    fn new(network: NetworkDecoder) -> Self
    where
        Self: Sized;

    fn config(&self) -> ConfigDecoder {
        let network = self.network();

        let p2pkh_prefix = match network {
            NetworkDecoder::Mainnet => self.get_pubkey_address_prefix_main(),
            _ => self.get_pubkey_address_prefix_test(),
        };
        let p2sh_prefix = match network {
            NetworkDecoder::Mainnet => self.get_script_address_prefix_main(),
            _ => self.get_script_address_prefix_test(),
        };
        let bech32_hrp = match network {
            NetworkDecoder::Mainnet => self.get_bech32_main(),
            NetworkDecoder::Testnet | NetworkDecoder::Signet => self.get_bech32_test(),
            NetworkDecoder::Regtest => self.get_bech32_reg(),
        }
        .to_string();

        ConfigDecoder {
            p2pkh_prefix,
            p2sh_prefix,
            bech32_hrp,
        }
    }

    fn encode(&self, payload: &bellscoin::address::Payload) -> String;
}

pub trait Decoder: Encoder + Send + Sync {
    fn decode(&self, address: &str) -> Result<Payload, bellscoin::address::Error> {
        let config = self.config();
        let bech32_prefix = match address.rfind('1') {
            None => address,
            Some(sep) => address.split_at(sep).0,
        };

        // try parse if is bech32 address
        if bech32_prefix == config.bech32_hrp {
            // decode as bech32
            let (_, payload, variant) = bech32::decode(address)?;
            if payload.is_empty() {
                return Err(bellscoin::address::Error::EmptyBech32Payload);
            }

            // Get the script version and program (converted from 5-bit to 8-bit)
            let (version, program): (WitnessVersion, Vec<u8>) = {
                let (v, p5) = payload.split_at(1);
                (
                    WitnessVersion::try_from(v[0])?,
                    bech32::FromBase32::from_base32(p5)?,
                )
            };

            let witness_program = WitnessProgram::new(version, program)?;

            // Encoding check
            let expected = version.bech32_variant();
            if expected != variant {
                return Err(bellscoin::address::Error::InvalidBech32Variant {
                    expected,
                    found: variant,
                });
            }

            return Ok(Payload::WitnessProgram(witness_program));
        }

        // Base58
        if address.len() > 50 {
            return Err(bellscoin::address::Error::Base58(
                base58::Error::InvalidLength(address.len() * 11 / 15),
            ));
        }

        let data = base58::decode_check(address)?;
        if data.len() != 21 {
            return Err(bellscoin::address::Error::Base58(
                base58::Error::InvalidLength(data.len()),
            ));
        }

        let payload = match data[0] {
            prefix if prefix == config.p2pkh_prefix => {
                Payload::PubkeyHash(PubkeyHash::from_slice(&data[1..]).unwrap())
            }
            prefix if prefix == config.p2sh_prefix => {
                Payload::ScriptHash(ScriptHash::from_slice(&data[1..]).unwrap())
            }
            x => {
                return Err(bellscoin::address::Error::Base58(
                    base58::Error::InvalidAddressVersion(x),
                ))
            }
        };

        Ok(payload)
    }
}

pub struct BellscoinDecoder(NetworkDecoder);

impl EncoderParams for BellscoinDecoder {
    fn get_pubkey_address_prefix_main(&self) -> u8 {
        bellscoin::blockdata::constants::PUBKEY_ADDRESS_PREFIX_MAIN
    }

    fn get_script_address_prefix_main(&self) -> u8 {
        bellscoin::blockdata::constants::SCRIPT_ADDRESS_PREFIX_MAIN
    }

    fn get_pubkey_address_prefix_test(&self) -> u8 {
        bellscoin::blockdata::constants::PUBKEY_ADDRESS_PREFIX_TEST
    }

    fn get_script_address_prefix_test(&self) -> u8 {
        bellscoin::blockdata::constants::SCRIPT_ADDRESS_PREFIX_TEST
    }

    fn get_bech32_main(&self) -> String {
        "bel".to_string()
    }

    fn get_bech32_test(&self) -> String {
        "tbel".to_string()
    }

    fn get_bech32_reg(&self) -> String {
        "bcrt".to_string()
    }

    fn network(&self) -> &NetworkDecoder {
        &self.0
    }
}

impl Encoder for BellscoinDecoder {
    fn encode(&self, payload: &bellscoin::address::Payload) -> String {
        let config = self.config();
        let encoding = bellscoin::address::AddressEncoding {
            payload,
            p2pkh_prefix: config.p2pkh_prefix,
            p2sh_prefix: config.p2sh_prefix,
            bech32_hrp: &config.bech32_hrp,
        };

        encoding.to_string()
    }

    fn new(network: NetworkDecoder) -> Self {
        Self(network)
    }
}

impl Decoder for BellscoinDecoder {}

pub struct DogecoinDecoder(NetworkDecoder);

impl EncoderParams for DogecoinDecoder {
    fn get_pubkey_address_prefix_main(&self) -> u8 {
        nintondo_dogecoin::blockdata::constants::PUBKEY_ADDRESS_PREFIX_MAIN
    }

    fn get_script_address_prefix_main(&self) -> u8 {
        nintondo_dogecoin::blockdata::constants::SCRIPT_ADDRESS_PREFIX_MAIN
    }

    fn get_pubkey_address_prefix_test(&self) -> u8 {
        nintondo_dogecoin::blockdata::constants::PUBKEY_ADDRESS_PREFIX_TEST
    }

    fn get_script_address_prefix_test(&self) -> u8 {
        nintondo_dogecoin::blockdata::constants::SCRIPT_ADDRESS_PREFIX_TEST
    }

    fn get_bech32_main(&self) -> String {
        "bc".to_string()
    }

    fn get_bech32_test(&self) -> String {
        "tb".to_string()
    }

    fn get_bech32_reg(&self) -> String {
        "bcrt".to_string()
    }

    fn network(&self) -> &NetworkDecoder {
        &self.0
    }
}

impl Encoder for DogecoinDecoder {
    fn encode(&self, payload: &bellscoin::address::Payload) -> String {
        let config = self.config();
        let encoding = bellscoin::address::AddressEncoding {
            payload,
            p2pkh_prefix: config.p2pkh_prefix,
            p2sh_prefix: config.p2sh_prefix,
            bech32_hrp: &config.bech32_hrp,
        };

        encoding.to_string()
    }

    fn new(network: NetworkDecoder) -> Self {
        Self(network)
    }
}

impl Decoder for DogecoinDecoder {}
