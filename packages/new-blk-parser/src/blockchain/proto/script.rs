use super::*;

use std::error::Error;

use bellscoin::address::{Payload, WitnessProgram, WitnessVersion};
use bellscoin::blockdata::script::Instruction;
use bellscoin::hashes::{Hash, sha256d};
use bellscoin::opcodes::Class::{IllegalOp, ReturnOp};
use bellscoin::{PublicKey, Script, opcodes};
use dutils::error::ContextWrapper;

use crate::blockchain::CoinType;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScriptError {
    UnexpectedEof,
    InvalidFormat,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let str = match *self {
            ScriptError::UnexpectedEof => "Unexpected EOF",
            ScriptError::InvalidFormat => "Invalid Script format",
        };
        write!(f, "{}", str)
    }
}

impl Error for ScriptError {}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ScriptPattern {
    /// Null Data
    /// Pubkey Script: OP_RETURN <0 to 80 bytes of data> (formerly 40 bytes)
    /// Null data scripts cannot be spent, so there's no signature script.
    OpReturn(String),

    /// Pay to Multisig [BIP11]
    /// Pubkey script: <m> <A pubkey>[B pubkey][C pubkey...] <n> OP_CHECKMULTISIG
    /// Signature script: OP_0 <A sig>[B sig][C sig...]
    Pay2MultiSig,

    /// Pay to Public Key (p2pk) scripts are a simplified form of the p2pkh,
    /// but aren't commonly used in new transactions anymore,
    /// because p2pkh scripts are more secure (the public key is not revealed until the output is spent).
    Pay2PublicKey,

    /// Pay to Public Key Hash (p2pkh)
    /// This is the most commonly used transaction output script.
    /// It's used to pay to a bitcoin address (a bitcoin address is a public key hash encoded in base58check)
    Pay2PublicKeyHash,

    /// Pay to Script Hash [p2sh/BIP16]
    /// The redeem script may be any pay type, but only multisig makes sense.
    /// Pubkey script: OP_HASH160 <Hash160(redeemScript)> OP_EQUAL
    /// Signature script: <sig>[sig][sig...] <redeemScript>
    Pay2ScriptHash,

    Pay2WitnessPublicKeyHash,

    Pay2WitnessScriptHash,

    WitnessProgram,

    /// A Taproot output is a native SegWit output (see BIP141) with version number 1, and a 32-byte witness program.
    /// See https://github.com/bitcoin/bips/blob/master/bip-0341.mediawiki#constructing-and-spending-taproot-outputs
    Pay2Taproot,

    Unspendable,

    /// The script is valid but does not conform to the standard templates.
    /// Such scripts are always accepted if they are mined into blocks, but
    /// transactions with non-standard scripts may not be forwarded by peers.
    NotRecognised,

    Error(ScriptError),
}

impl fmt::Display for ScriptPattern {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ScriptPattern::OpReturn(_) => write!(f, "OpReturn"),
            ScriptPattern::Pay2MultiSig => write!(f, "Pay2MultiSig"),
            ScriptPattern::Pay2PublicKey => write!(f, "Pay2PublicKey"),
            ScriptPattern::Pay2PublicKeyHash => write!(f, "Pay2PublicKeyHash"),
            ScriptPattern::Pay2ScriptHash => write!(f, "Pay2ScriptHash"),
            ScriptPattern::Pay2WitnessPublicKeyHash => write!(f, "Pay2WitnessPublicKeyHash"),
            ScriptPattern::Pay2WitnessScriptHash => write!(f, "Pay2WitnessScriptHash"),
            ScriptPattern::WitnessProgram => write!(f, "WitnessProgram"),
            ScriptPattern::Pay2Taproot => write!(f, "Pay2Taproot"),
            ScriptPattern::Unspendable => write!(f, "Unspendable"),
            ScriptPattern::NotRecognised => write!(f, "NotRecognised"),
            ScriptPattern::Error(ref err) => write!(f, "ScriptError: {}", err),
        }
    }
}

#[derive(Clone)]
pub struct EvaluatedScript {
    pub address: Option<String>,
    pub pattern: ScriptPattern,
}

impl EvaluatedScript {
    #[inline]
    pub fn new(address: Option<String>, pattern: ScriptPattern) -> Self {
        Self { address, pattern }
    }
}

/// Extracts evaluated address from ScriptPubKey
#[inline]
pub fn eval_from_bytes(bytes: &[u8], coin: CoinType) -> EvaluatedScript {
    eval_from_bytes_bellscoin(bytes, coin)
}

/// Extracts evaluated address from script using `rust_bitcoin`
pub fn eval_from_bytes_bellscoin(bytes: &[u8], coin: CoinType) -> EvaluatedScript {
    let script = Script::from_bytes(bytes);

    // For OP_RETURN and provably unspendable scripts there is no point in parsing the address
    if script.is_op_return() {
        // OP_RETURN 13 <data>
        let data = String::from_utf8(script.to_bytes().into_iter().skip(2).collect());
        let pattern = ScriptPattern::OpReturn(data.unwrap_or_else(|_| String::from("")));
        return EvaluatedScript::new(None, pattern);
    } else if is_provable_unspendable(script) {
        return EvaluatedScript::new(None, ScriptPattern::Unspendable);
    }

    let address = script_to_address_str(script, coin);

    if script.is_p2pk() {
        EvaluatedScript::new(p2pk_to_string(script, coin), ScriptPattern::Pay2PublicKey)
    } else if script.is_p2pkh() {
        EvaluatedScript::new(address, ScriptPattern::Pay2PublicKeyHash)
    } else if script.is_p2sh() {
        EvaluatedScript::new(address, ScriptPattern::Pay2ScriptHash)
    } else if script.is_v0_p2wpkh() {
        EvaluatedScript::new(address, ScriptPattern::Pay2WitnessPublicKeyHash)
    } else if script.is_v0_p2wsh() {
        EvaluatedScript::new(address, ScriptPattern::Pay2WitnessScriptHash)
    } else if script.is_v1_p2tr() {
        EvaluatedScript::new(address, ScriptPattern::Pay2Taproot)
    } else if script.is_witness_program() {
        EvaluatedScript::new(address, ScriptPattern::WitnessProgram)
    } else {
        EvaluatedScript::new(address, ScriptPattern::NotRecognised)
    }
}

pub fn script_to_address_str(script: &Script, coin: CoinType) -> Option<String> {
    match Payload::from_script(script) {
        Ok(payload) => Some(payload_to_address_str(payload, coin)),
        Err(err) => {
            if err != bellscoin::address::Error::UnrecognizedScript {
                warn!(target: "script", "Unable to extract evaluated address: {}", err)
            }
            None
        }
    }
}

pub fn payload_to_address_str(payload: Payload, coin: CoinType) -> String {
    bellscoin::address::AddressEncoding {
        payload: &payload,
        p2pkh_prefix: coin.pubkey_address,
        p2sh_prefix: coin.script_address,
        bech32_hrp: coin.bech32,
    }
    .to_string()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ScriptType {
    Address,
    ScriptHash,
}

impl FromStr for ScriptType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        serde_json::from_str(&format!("\"{}\"", s)).anyhow_with("Invalid script type")
    }
}

pub fn address_to_payload(address: &str, coin: CoinType) -> crate::Result<Payload> {
    let is_bech32 = address
        .rfind('1')
        .map(|x| address.split_at(x).0)
        .is_some_and(|v| v == coin.bech32);

    if is_bech32 {
        let (_, payload, variant) = bellscoin::bech32::decode(address)?;
        if payload.is_empty() {
            anyhow::bail!("Empty bech32 payload");
        }

        // Get the script version and program (converted from 5-bit to 8-bit)
        let (version, program): (WitnessVersion, Vec<u8>) = {
            let (v, p5) = payload.split_at(1);
            (
                WitnessVersion::try_from(v[0])?,
                bellscoin::bech32::FromBase32::from_base32(p5)?,
            )
        };

        let witness_program = WitnessProgram::new(version, program)?;

        // Encoding check
        let expected = version.bech32_variant();
        if expected != variant {
            anyhow::bail!(
                "Invalid bech32 variant. Expected: {:?}, found: {:?}",
                expected,
                variant
            );
        }

        return Ok(Payload::WitnessProgram(witness_program));
    }

    // Base58
    if address.len() > 50 {
        anyhow::bail!("Invalid address length");
    }

    let data = bellscoin::base58::decode_check(address)?;
    if data.len() != 21 {
        anyhow::bail!("Invalid address length");
    }

    let payload = match data[0] {
        v if v == coin.pubkey_address => {
            Payload::PubkeyHash(bellscoin::PubkeyHash::from_slice(&data[1..]).unwrap())
        }
        v if v == coin.script_address => {
            Payload::ScriptHash(bellscoin::ScriptHash::from_slice(&data[1..]).unwrap())
        }
        _ => anyhow::bail!("Invalid address version"),
    };

    Ok(payload)
}

pub fn address_to_fullhash(
    address: &str,
    script_type: ScriptType,
    coin: CoinType,
) -> crate::Result<sha256d::Hash> {
    match script_type {
        ScriptType::Address => {
            let payload = address_to_payload(address, coin)?.script_pubkey();
            Ok(sha256d::Hash::hash(payload.as_bytes()))
        }
        ScriptType::ScriptHash => {
            sha256d::Hash::from_slice(&hex::decode(address).anyhow_with("Invalid hex")?)
                .anyhow_with("Invalid script hash length")
        }
    }
}

/// Workaround to parse address from p2pk scripts
/// See issue https://github.com/rust-bitcoin/rust-bitcoin/issues/441
fn p2pk_to_string(script: &Script, coin: CoinType) -> Option<String> {
    debug_assert!(script.is_p2pk());
    let pk = match script.instructions().next() {
        Some(Ok(Instruction::PushBytes(bytes))) => bytes,
        Some(Err(msg)) => {
            warn!(target: "script", "Unable to parse address from p2pk script: {}", msg);
            return None;
        }
        _ => unreachable!(),
    };

    let payload = Payload::p2pkh(&PublicKey::from_slice(pk.as_bytes()).ok()?);
    Some(payload_to_address_str(payload, coin))
}

/// Checks whether a script is trivially known to have no satisfying input.
#[inline]
fn is_provable_unspendable(script: &Script) -> bool {
    match script.as_bytes().first() {
        Some(b) => {
            let first = bellscoin::opcodes::All::from(*b);
            let class = first.classify(opcodes::ClassifyContext::Legacy);

            class == ReturnOp || class == IllegalOp
        }
        None => false,
    }
}
