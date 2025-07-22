use bellscoin::ScriptBuf;
use nint_blk::proto::block::Block;

use super::{processe_data::ProcessedData, *};

pub fn process_prevouts(db: Arc<DB>, block: &Block, data_to_write: &mut Vec<ProcessedData>) -> anyhow::Result<HashMap<OutPoint, TxPrevout>> {
    let prevouts = block
        .txs
        .iter()
        .flat_map(|tx| {
            let txid = tx.hash;
            tx.value.outputs.iter().enumerate().map(move |(vout, txout)| {
                (
                    OutPoint {
                        txid: txid.into(),
                        vout: vout as u32,
                    },
                    TxOut {
                        value: txout.out.value,
                        script_pubkey: ScriptBuf::from_bytes(txout.out.script_pubkey.clone()),
                    },
                )
            })
        })
        .filter(|(_, txout)| !txout.script_pubkey.is_provably_unspendable())
        .map(|(outpoint, tx_out)| (outpoint, tx_out.into()))
        .collect::<HashMap<_, TxPrevout>>();

    let txids_keys = block
        .txs
        .iter()
        .filter(|tx| !tx.value.is_coinbase())
        .flat_map(|tx| tx.value.inputs.iter().map(|x| x.outpoint))
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
                        result.insert(*key, *value);
                    } else {
                        return Err(anyhow::anyhow!("Missing prevout for key {}", key));
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
