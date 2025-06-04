use super::{processe_data::ProcessedData, *};

pub fn load_prevouts_for_block(
    db: Arc<DB>,
    txs: &[Transaction],
    data_to_write: &mut Vec<ProcessedData>,
) -> anyhow::Result<HashMap<OutPoint, TxOut>> {
    let prevouts = txs
        .iter()
        .flat_map(|tx| {
            let txid = tx.txid();
            tx.output
                .iter()
                .enumerate()
                .map(move |(input_index, txout)| {
                    (
                        OutPoint {
                            txid,
                            vout: input_index as u32,
                        },
                        txout.clone(),
                    )
                })
        })
        .filter(|(_, txout)| !txout.script_pubkey.is_provably_unspendable())
        .collect::<HashMap<_, _>>();

    let txids_keys = txs
        .iter()
        .skip(1)
        .flat_map(|x| x.input.iter().map(|x| x.previous_output))
        .unique()
        .collect_vec();

    let mut result = HashMap::new();

    if !txids_keys.is_empty() {
        let from_db = db.prevouts.multi_get(txids_keys.iter());

        for (key, maybe_val) in txids_keys.iter().zip(from_db) {
            match maybe_val {
                Some(val) => {
                    result.insert(*key, val);
                }
                None => {
                    if let Some(value) = prevouts.get(key) {
                        result.insert(*key, value.clone());
                    } else {
                        return Err(anyhow::anyhow!("Missing prevout for key: {:?}", key));
                    }
                }
            }
        }
    }

    data_to_write.push(ProcessedData::Prevouts {
        to_write: prevouts,
        to_remove: txids_keys,
    });

    Ok(result)
}
