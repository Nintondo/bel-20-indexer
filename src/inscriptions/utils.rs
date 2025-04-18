use super::*;

pub fn load_prevouts_for_block(
    db: Arc<DB>,
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

    let prevouts = db
        .prevouts
        .multi_get(txids_keys.iter())
        .into_iter()
        .zip(txids_keys.clone())
        .map(|(v, k)| v.map(|x| (k, x)))
        .collect::<Option<HashMap<_, _>>>()
        .anyhow_with("Some prevouts are missing")?;

    std::thread::spawn(move || {
        db.prevouts.remove_batch(txids_keys.iter());
    });

    Ok(prevouts)
}
