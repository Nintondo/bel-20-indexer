use nint_blk::proto::{
    tx::{EvaluatedTx, EvaluatedTxOut},
    Hashed,
};

use super::*;
use hashbrown::HashMap;

pub struct InscriptionSearcher {}

impl InscriptionSearcher {
    pub fn calc_offsets(tx: &Hashed<EvaluatedTx>, tx_outs: &HashMap<OutPoint, TxPrevout>) -> Option<Vec<u64>> {
        let mut input_values = tx.value.inputs.iter().map(|x| tx_outs.get(&x.outpoint).map(|x| x.value)).collect::<Option<Vec<u64>>>()?;
        let spend: u64 = input_values.iter().sum();

        let mut fee = spend - tx.value.outputs.iter().map(|x| x.out.value).sum::<u64>();
        while let Some(input) = input_values.pop() {
            if input > fee {
                input_values.push(input - fee);
                break;
            }
            fee -= input;
        }

        let mut inputs_offsets = input_values.iter().fold(vec![0], |mut acc, x| {
            acc.push(acc.last().unwrap() + x);
            acc
        });

        inputs_offsets.pop();

        Some(inputs_offsets)
    }

    pub fn get_output_index_by_input(offset: Option<u64>, tx_outs: &[EvaluatedTxOut]) -> anyhow::Result<(u32, u64)> {
        let cum = Self::calc_output_prefixes(tx_outs);
        Self::get_output_index_by_input_with_prefix(offset, &cum)
    }

    /// Precompute cumulative output values once per transaction so that multiple
    /// offset lookups can reuse the same prefix sums.
    pub fn calc_output_prefixes(tx_outs: &[EvaluatedTxOut]) -> Vec<u64> {
        let mut cum = Vec::with_capacity(tx_outs.len());
        let mut acc: u64 = 0;
        for o in tx_outs {
            acc = acc.saturating_add(o.out.value);
            cum.push(acc);
        }
        cum
    }

    /// Map a global output offset to (vout index, offset inside vout) using a
    /// precomputed prefix-sum array produced by `calc_output_prefixes`.
    pub fn get_output_index_by_input_with_prefix(offset: Option<u64>, out_cum: &[u64]) -> anyhow::Result<(u32, u64)> {
        let Some(offset) = offset else {
            return Err(anyhow::anyhow!("leaked: offset is None"));
        };

        if out_cum.is_empty() {
            return Err(anyhow::anyhow!("leaked: offset={} is too large for total_output=0", offset));
        }

        let total_output: u64 = *out_cum.last().unwrap();
        if offset >= total_output {
            return Err(anyhow::anyhow!("leaked: offset={} is too large for total_output={}", offset, total_output));
        }

        let mut prev_bound: u64 = 0;
        for (idx, bound) in out_cum.iter().copied().enumerate() {
            if offset < bound {
                return Ok((idx as u32, offset - prev_bound));
            }
            prev_bound = bound;
        }

        Err(anyhow::anyhow!("leaked: offset exhausted"))
    }
}
