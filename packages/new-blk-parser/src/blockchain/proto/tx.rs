use super::*;

use bellscoin::{OutPoint, Witness};
use proto::{Hashable, ToRaw, script, varuint::VarUint};
use std::io::Write;

pub struct RawTx {
    pub version: u32,
    pub in_count: VarUint,
    pub inputs: Vec<TxInput>,
    pub out_count: VarUint,
    pub outputs: Vec<TxOutput>,
    pub locktime: u32,
    pub coin: CoinType,
}

/// Simple transaction struct
/// Please note: The txid is not stored here. See Hashed.
#[derive(Clone)]
pub struct EvaluatedTx {
    pub version: u32,
    pub in_count: VarUint,
    pub inputs: Vec<TxInput>,
    pub out_count: VarUint,
    pub outputs: Vec<EvaluatedTxOut>,
    pub locktime: u32,
}

impl EvaluatedTx {
    pub fn new(version: u32, in_count: VarUint, inputs: Vec<TxInput>, out_count: VarUint, outputs: Vec<TxOutput>, locktime: u32, coin: CoinType) -> Self {
        // Evaluate and wrap all outputs to process them later
        let outputs = outputs.into_par_iter().map(|o| EvaluatedTxOut::eval_script(o, coin)).collect();
        EvaluatedTx {
            version,
            in_count,
            inputs,
            out_count,
            outputs,
            locktime,
        }
    }

    #[inline]
    pub fn is_coinbase(&self) -> bool {
        if self.in_count.value == 1 {
            let input = self.inputs.first().unwrap();
            return input.outpoint.txid.as_ref() == [0u8; 32] && input.outpoint.vout == 0xFFFFFFFF;
        }
        false
    }
}

impl fmt::Debug for EvaluatedTx {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("Tx")
            .field("version", &self.version)
            .field("in_count", &self.in_count)
            .field("out_count", &self.out_count)
            .field("locktime", &self.locktime)
            .finish()
    }
}

impl From<RawTx> for EvaluatedTx {
    #[inline]
    fn from(tx: RawTx) -> Self {
        Self::new(tx.version, tx.in_count, tx.inputs, tx.out_count, tx.outputs, tx.locktime, tx.coin)
    }
}

impl ToRaw for EvaluatedTx {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity((4 + self.in_count.value + self.out_count.value + 4) as usize);

        // Serialize version
        bytes.extend(&self.version.to_le_bytes());
        // Serialize all TxInputs
        bytes.extend(&self.in_count.to_bytes());
        for i in &self.inputs {
            bytes.extend(&i.to_bytes());
        }
        // Serialize all TxOutputs
        bytes.extend(&self.out_count.to_bytes());
        for o in &self.outputs {
            bytes.extend(&o.out.to_bytes());
        }
        // Serialize locktime
        bytes.extend(&self.locktime.to_le_bytes());
        bytes
    }
}

impl Hashable for EvaluatedTx {
    fn hash_to_engine<W: Write>(&self, engine: &mut W) {
        // Version
        let _ = engine.write_all(&self.version.to_le_bytes());

        // Inputs (count + each input)
        let _ = engine.write_all(&self.in_count.to_bytes());
        for input in &self.inputs {
            // OutPoint (txid + vout)
            let _ = engine.write_all(&input.outpoint.txid.to_byte_array());
            let _ = engine.write_all(&input.outpoint.vout.to_le_bytes());
            // script length + script bytes
            let _ = engine.write_all(&input.script_len.to_bytes());
            let _ = engine.write_all(&input.script_sig);
            // sequence
            let _ = engine.write_all(&input.seq_no.to_le_bytes());
        }

        // Outputs (count + each output)
        let _ = engine.write_all(&self.out_count.to_bytes());
        for eval_out in &self.outputs {
            let out = &eval_out.out;
            let _ = engine.write_all(&out.value.to_le_bytes());
            let _ = engine.write_all(&out.script_len.to_bytes());
            let _ = engine.write_all(&out.script_pubkey);
        }

        // Locktime
        let _ = engine.write_all(&self.locktime.to_le_bytes());
    }
}

/// TxOutpoint references an existing transaction output
#[derive(PartialEq, Eq, Hash, Clone)]
pub struct TxOutpoint {
    pub txid: sha256d::Hash,
    pub index: u32, // 0-based offset within tx
}

impl TxOutpoint {
    #[inline]
    pub fn new(txid: sha256d::Hash, index: u32) -> Self {
        Self { txid, index }
    }
}

impl ToRaw for TxOutpoint {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(32 + 4);
        bytes.extend(self.txid.as_byte_array());
        bytes.extend(&self.index.to_le_bytes());
        bytes
    }
}

impl fmt::Debug for TxOutpoint {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("TxOutpoint").field("txid", &self.txid).field("index", &self.index).finish()
    }
}

/// Holds TxInput informations
#[derive(Clone)]
pub struct TxInput {
    pub outpoint: OutPoint,
    pub script_len: VarUint,
    pub script_sig: Vec<u8>,
    pub seq_no: u32,
    pub witness: Witness,
}

impl ToRaw for TxInput {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(36 + 5 + self.script_len.value as usize + 4);
        bytes.extend(&self.outpoint.txid.to_byte_array());
        bytes.extend(&self.outpoint.vout.to_le_bytes());
        bytes.extend(&self.script_len.to_bytes());
        bytes.extend(&self.script_sig);
        bytes.extend(&self.seq_no.to_le_bytes());
        bytes
    }
}

impl fmt::Debug for TxInput {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("TxInput")
            .field("outpoint", &self.outpoint)
            .field("script_len", &self.script_len)
            .field("script_sig", &self.script_sig)
            .field("seq_no", &self.seq_no)
            .finish()
    }
}

/// Evaluates script_pubkey and wraps TxOutput
#[derive(Clone)]
pub struct EvaluatedTxOut {
    pub script: script::EvaluatedScript,
    pub out: TxOutput,
}

impl EvaluatedTxOut {
    #[inline]
    pub fn eval_script(out: TxOutput, coin: CoinType) -> EvaluatedTxOut {
        // For pre-FIB fast-sync we can skip full script evaluation, as
        // inscriptions logic never inspects addresses or patterns there.
        if !crate::is_script_eval_enabled() {
            let script = script::EvaluatedScript::new(None, script::ScriptPattern::NotRecognised);
            return EvaluatedTxOut { script, out };
        }

        use crate::timing::BLOCK_READ_METRICS;
        let start = std::time::Instant::now();
        let script = script::eval_from_bytes(&out.script_pubkey, coin);
        BLOCK_READ_METRICS.record_script_eval(start.elapsed());
        EvaluatedTxOut { script, out }
    }
}

/// Holds TxOutput informations
#[derive(Clone)]
pub struct TxOutput {
    pub value: u64,
    pub script_len: VarUint,
    pub script_pubkey: Vec<u8>,
}

impl ToRaw for TxOutput {
    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8 + 5 + self.script_len.value as usize);
        bytes.extend(&self.value.to_le_bytes());
        bytes.extend(&self.script_len.to_bytes());
        bytes.extend(&self.script_pubkey);
        bytes
    }
}

impl fmt::Debug for TxOutput {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        fmt.debug_struct("TxOutput")
            .field("value", &self.value)
            .field("script_len", &self.script_len)
            .field("script_pubkey", &utils::arr_to_hex(&self.script_pubkey))
            .finish()
    }
}
