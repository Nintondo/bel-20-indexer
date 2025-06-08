use nint_blk::proto::{tx::EvaluatedTx, Hashed};

use super::*;

#[derive(Clone)]
pub enum LeakedInscription {
    Creation,
    Move,
}

pub struct LeakedInscriptions {
    pub inscriptions: HashMap<u64, Vec<LeakedInscription>>,
    pub total_amount: u64,
    pub coinbase_tx: Hashed<EvaluatedTx>,
    pub coinbase_reward: Option<u64>,
}

struct FeeResult {
    fee: u64,
    fee_offset: u64,
}

impl LeakedInscriptions {
    pub fn new(coinbase_tx: Hashed<EvaluatedTx>) -> Self {
        Self {
            coinbase_tx,
            inscriptions: HashMap::new(),
            total_amount: 0,
            coinbase_reward: None,
        }
    }

    pub fn add(
        &mut self,
        input_idx: usize,
        tx: &Hashed<EvaluatedTx>,
        input_offset: u64,
        tx_outs: &HashMap<OutPoint, TxOut>,
        inscription: LeakedInscription,
    ) {
        let fee_result = Self::find_fee(tx, input_idx, input_offset, tx_outs);

        let diff = fee_result.fee - fee_result.fee_offset;

        self.inscriptions
            .entry(self.total_amount - diff)
            .and_modify(|x| {
                x.push(inscription.clone());
            })
            .or_insert(vec![inscription]);
    }

    pub fn add_tx_fee(&mut self, tx: &Hashed<EvaluatedTx>, txos: &HashMap<OutPoint, TxOut>) -> u64 {
        let inputs_sum = tx
            .value
            .inputs
            .iter()
            .map(|x| txos.get(&x.outpoint).unwrap().value)
            .sum::<u64>();

        let outputs_sum = tx.value.outputs.iter().map(|x| x.out.value).sum::<u64>();

        self.total_amount += inputs_sum - outputs_sum;

        self.total_amount
    }

    fn update_reward(&mut self) {
        self.coinbase_reward = Some(
            self.coinbase_tx
                .value
                .outputs
                .iter()
                .map(|x| x.out.value)
                .sum::<u64>()
                - self.total_amount,
        );
    }

    pub fn get_leaked_inscriptions(mut self) -> impl Iterator<Item = Location> {
        self.update_reward();

        self.inscriptions
            .clone()
            .into_iter()
            .flat_map(|(offset, x)| x.into_iter().map(move |x| (offset, x)))
            .filter_map(move |(offset, _)| {
                self.find_inscription_vout(offset)
                    .map(|(vout, offset)| Location {
                        offset,
                        outpoint: OutPoint {
                            txid: self.coinbase_tx.hash.into(),
                            vout,
                        },
                    })
            })
    }

    fn find_inscription_vout(&self, offset: u64) -> Option<(u32, u64)> {
        let mut offset = offset + self.coinbase_reward.unwrap();

        for (i, tx) in self.coinbase_tx.value.outputs.iter().enumerate() {
            if offset < tx.out.value {
                return Some((i as u32, offset));
            }
            offset -= tx.out.value;
        }
        None
    }

    fn find_fee(
        tx: &Hashed<EvaluatedTx>,
        input_idx: usize,
        input_offset: u64,
        tx_outs: &HashMap<OutPoint, TxOut>,
    ) -> FeeResult {
        let inputs_cum = {
            let mut last_value = 0;

            tx.value
                .inputs
                .iter()
                .map(|x| {
                    last_value += tx_outs.get(&x.outpoint).unwrap().value;
                    last_value
                })
                .collect_vec()
        };

        let output_sum = tx.value.outputs.iter().map(|x| x.out.value).sum::<u64>();
        let input_sum = *inputs_cum.last().unwrap();

        let prev_out_value = tx_outs
            .get(&tx.value.inputs.get(input_idx).unwrap().outpoint)
            .map(|x| x.value)
            .unwrap();

        let offset = inputs_cum[input_idx] - prev_out_value + input_offset - output_sum;

        FeeResult {
            fee: input_sum - output_sum,
            fee_offset: offset,
        }
    }
}
