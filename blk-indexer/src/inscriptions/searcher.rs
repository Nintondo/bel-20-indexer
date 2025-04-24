use std::collections::HashMap;
use bellscoin::{OutPoint, Transaction, TxOut};
use super::*;

pub struct InscriptionSearcher {}

impl InscriptionSearcher {
    pub fn calc_offsets(tx: &Transaction, tx_outs: &HashMap<OutPoint, TxOut>) -> Option<Vec<u64>> {
        let mut input_values = tx
            .input
            .iter()
            .map(|x| tx_outs.get(&x.previous_output).map(|x| x.value))
            .collect::<Option<Vec<u64>>>()?;

        let spend: u64 = input_values.iter().sum();

        let mut fee = spend - tx.output.iter().map(|x| x.value).sum::<u64>();
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

    pub fn get_output_index_by_input(
        offset: Option<u64>,
        tx_outs: &[TxOut],
    ) -> anyhow::Result<(u32, u64)> {
        let Some(mut offset) = offset else {
            anyhow::bail!("leaked");
        };

        for (idx, out) in tx_outs.iter().enumerate() {
            if offset < out.value {
                return Ok((idx as u32, offset));
            }
            offset -= out.value;
        }

        anyhow::bail!("leaked");
    }
}
