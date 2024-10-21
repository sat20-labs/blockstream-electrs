use crate::chain::{BlockHash, OutPoint, Transaction, TxIn, TxOut, Txid};
use crate::util::BlockId;

use std::collections::{BTreeSet, HashMap};

#[derive(Serialize, Deserialize, Debug)]
pub struct TransactionStatus {
    pub confirmed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_height: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<BlockHash>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_time: Option<u32>,
}

impl From<Option<BlockId>> for TransactionStatus {
    fn from(blockid: Option<BlockId>) -> TransactionStatus {
        match blockid {
            Some(b) => TransactionStatus {
                confirmed: true,
                block_height: Some(b.height as usize),
                block_hash: Some(b.hash),
                block_time: Some(b.time),
            },
            None => TransactionStatus {
                confirmed: false,
                block_height: None,
                block_hash: None,
                block_time: None,
            },
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct TxInput {
    pub txid: Txid,
    pub vin: u16,
}

pub fn is_coinbase(txin: &TxIn) -> bool {
    return txin.previous_output.is_null();
}

pub fn has_prevout(txin: &TxIn) -> bool {
    return !txin.previous_output.is_null();
}

pub fn is_spendable(txout: &TxOut) -> bool {
    // return !txout.script_pubkey.is_provably_unspendable();
    return !txout.script_pubkey.is_op_return();
}

pub fn extract_tx_prevouts<'a>(
    tx: &Transaction,
    txos: &'a HashMap<OutPoint, TxOut>,
    allow_missing: bool,
) -> HashMap<u32, &'a TxOut> {
    let skip_output = crate::new_index::schema::get_skip_outpoint();
    tx.input
        .iter()
        .enumerate()
        .filter(|(_, txi)| has_prevout(txi) && txi.previous_output != skip_output)
        .filter_map(|(index, txi)| {
            Some((
                index as u32,
                txos.get(&txi.previous_output).or_else(|| {
                    // assert!(allow_missing, "missing outpoint {:?}", txi.previous_output);
                    if !allow_missing {
                        error!("Missing outpoint: {:?}", txi.previous_output);
                        debug_assert!(false, "missing outpoint {:?}", txi.previous_output);
                    }
                    None
                })?,
            ))
        })
        .collect()
}

pub fn get_prev_outpoints<'a>(txs: impl Iterator<Item = &'a Transaction>) -> BTreeSet<OutPoint> {
    txs.flat_map(|tx| {
        tx.input
            .iter()
            .filter(|txin| has_prevout(txin))
            .map(|txin| txin.previous_output)
    })
    .collect()
}

pub fn serialize_outpoint<S>(outpoint: &OutPoint, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::ser::Serializer,
{
    use serde::ser::SerializeStruct;
    let mut s = serializer.serialize_struct("OutPoint", 2)?;
    s.serialize_field("txid", &outpoint.txid)?;
    s.serialize_field("vout", &outpoint.vout)?;
    s.end()
}
