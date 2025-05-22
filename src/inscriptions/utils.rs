use super::*;

pub fn load_prevouts_for_block(
    db: Arc<DB>,
    prevouts: HashMap<OutPoint, TxOut>,
    txs: &[Transaction],
) -> anyhow::Result<HashMap<OutPoint, TxOut>> {
    let txids_keys = txs
        .iter()
        .skip(1)
        .flat_map(|x| x.input.iter().map(|x| x.previous_output))
        .unique()
        .collect_vec();

    if txids_keys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut result = HashMap::new();
    let mut missing_keys = Vec::new();

    for key in &txids_keys {
        if let Some(value) = prevouts.get(key) {
            result.insert(*key, value.clone());
        } else {
            missing_keys.push(*key);
        }
    }

    if !missing_keys.is_empty() {
        let from_db = db.prevouts.multi_get(missing_keys.iter());
        for (key, maybe_val) in missing_keys.iter().zip(from_db) {
            match maybe_val {
                Some(val) => {
                    result.insert(*key, val);
                }
                None => {
                    return Err(anyhow::anyhow!("Missing prevout for key: {:?}", key));
                }
            }
        }
    }

    let db_clone = db.clone();
    let all_keys = txids_keys.clone();
    std::thread::spawn(move || {
        db_clone.prevouts.remove_batch(all_keys.iter());
    });

    Ok(result)
}
