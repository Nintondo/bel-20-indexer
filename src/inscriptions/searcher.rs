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
            return Err(anyhow::anyhow!("leaked: offset is None"));
        };

        let total_output: u64 = tx_outs.iter().map(|x| x.value).sum();
        if offset >= total_output {
            return Err(anyhow::anyhow!(
                "leaked: offset={} is too large for total_output={}",
                offset,
                total_output
            ));
        }

        for (idx, out) in tx_outs.iter().enumerate() {
            if offset < out.value {
                return Ok((idx as u32, offset));
            }
            offset -= out.value;
        }

        Err(anyhow::anyhow!("leaked: offset exhausted"))
    }
}
