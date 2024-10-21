use crate::chain::{
    address, BlockHash, Network, OutPoint, Script, Sequence, Transaction, TxIn, TxMerkleNode,
    TxOut, Txid,
};
use crate::config::Config;
use crate::errors;
use crate::new_index::{compute_script_hash, Query, SpendingInput, Utxo};
use crate::util::{
    create_socket, electrum_merkle, extract_tx_prevouts, get_innerscripts, get_tx_fee, has_prevout,
    is_coinbase, BlockHeaderMeta, BlockId, FullHash, ScriptToAddr, ScriptToAsm, TransactionStatus,
    DEFAULT_BLOCKHASH,
};

use satsnet::consensus::encode;

use satsnet::hashes::FromSliceError as HashError;
use hex::{DisplayHex, FromHex};
use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Response, Server, StatusCode};
use hyperlocal::UnixServerExt;
use tokio::sync::oneshot;

use std::fs;
use std::str::FromStr;

use serde::Serialize;
use serde_json;
use std::collections::HashMap;
use std::num::ParseIntError;
use std::os::unix::fs::FileTypeExt;
use std::sync::Arc;
use std::thread;
use url::form_urlencoded;

const CHAIN_TXS_PER_PAGE: usize = 25;
const MAX_MEMPOOL_TXS: usize = 50;
const BLOCK_LIMIT: usize = 10;
const ADDRESS_SEARCH_LIMIT: usize = 10;

const TTL_LONG: u32 = 157_784_630; // ttl for static resources (5 years)
const TTL_SHORT: u32 = 10; // ttl for volatie resources
const TTL_MEMPOOL_RECENT: u32 = 5; // ttl for GET /mempool/recent
const CONF_FINAL: usize = 10; // reorgs deeper than this are considered unlikely

#[derive(Serialize, Deserialize)]
struct BlockValue {
    id: BlockHash,
    height: u32,
    version: u32,
    timestamp: u32,
    tx_count: u32,
    size: u32,
    weight: u64,
    merkle_root: TxMerkleNode,
    previousblockhash: Option<BlockHash>,
    mediantime: u32,

    nonce: u32,
    bits: satsnet::pow::CompactTarget,
    difficulty: f64,
}

impl BlockValue {
    fn new(blockhm: BlockHeaderMeta) -> Self {
        let header = blockhm.header_entry.header();
        BlockValue {
            id: header.block_hash(),
            height: blockhm.header_entry.height() as u32,
            version: header.version.to_consensus() as u32,
            timestamp: header.time,
            tx_count: blockhm.meta.tx_count,
            size: blockhm.meta.size,
            weight: blockhm.meta.weight as u64,
            merkle_root: header.merkle_root,
            previousblockhash: if header.prev_blockhash != *DEFAULT_BLOCKHASH {
                Some(header.prev_blockhash)
            } else {
                None
            },
            mediantime: blockhm.mtp,

            bits: header.bits,
            nonce: header.nonce,
            difficulty: header.difficulty_float(),
        }
    }
}

#[derive(Serialize)]
struct TransactionValue {
    txid: Txid,
    version: u32,
    locktime: u32,
    vin: Vec<TxInValue>,
    vout: Vec<TxOutValue>,
    size: u32,
    weight: u64,
    fee: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<TransactionStatus>,
}

impl TransactionValue {
    fn new(
        tx: Transaction,
        blockid: Option<BlockId>,
        txos: &HashMap<OutPoint, TxOut>,
        config: &Config,
    ) -> Self {
        // let prevouts = extract_tx_prevouts(&tx, &txos, true);
        let prevouts = if txos.is_empty() {
            HashMap::new()
        } else {
            extract_tx_prevouts(&tx, txos, true)
        };
        
        let vins: Vec<TxInValue> = tx
            .input
            .iter()
            .enumerate()
            .map(|(index, txin)| {
                TxInValue::new(txin, prevouts.get(&(index as u32)).cloned(), config)
            })
            .collect();

        let vouts: Vec<TxOutValue> = tx
            .output
            .iter()
            .map(|txout| TxOutValue::new(txout, config))
            .collect();

        // let fee = get_tx_fee(&tx, &prevouts, config.network_type);
        let fee = if prevouts.len() == 0 {
            0
        } else {
            get_tx_fee(&tx, &prevouts, config.network_type)
        };

        let weight = tx.weight();
        // rust-bitcoin has a wrapper Weight type
        let weight = weight.to_wu();

        TransactionValue {
            // txid: tx.txid(),
            txid: tx.compute_txid(),
            version: tx.version.0 as u32,
            locktime: tx.lock_time.to_consensus_u32(),
            vin: vins,
            vout: vouts,
            size: tx.total_size() as u32,
            weight: weight as u64,
            fee,
            status: Some(TransactionStatus::from(blockid)),
        }
    }
}

#[derive(Serialize, Clone)]
struct TxInValue {
    txid: Txid,
    vout: u32,
    prevout: Option<TxOutValue>,
    scriptsig: Script,
    scriptsig_asm: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    witness: Option<Vec<String>>,
    is_coinbase: bool,
    sequence: Sequence,

    #[serde(skip_serializing_if = "Option::is_none")]
    inner_redeemscript_asm: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inner_witnessscript_asm: Option<String>,
}

impl TxInValue {
    fn new(txin: &TxIn, prevout: Option<&TxOut>, config: &Config) -> Self {
        let witness = &txin.witness;
        let witness = if !witness.is_empty() {
            Some(
                witness
                    .iter()
                    .map(DisplayHex::to_lower_hex_string)
                    .collect(),
            )
        } else {
            None
        };

        let is_coinbase = is_coinbase(&txin);

        let innerscripts = prevout.map(|prevout| get_innerscripts(&txin, &prevout));

        TxInValue {
            txid: txin.previous_output.txid,
            vout: txin.previous_output.vout,
            prevout: prevout.map(|prevout| TxOutValue::new(prevout, config)),
            scriptsig_asm: txin.script_sig.to_asm(),
            witness,

            inner_redeemscript_asm: innerscripts
                .as_ref()
                .and_then(|i| i.redeem_script.as_ref())
                .map(ScriptToAsm::to_asm),
            inner_witnessscript_asm: innerscripts
                .as_ref()
                .and_then(|i| i.witness_script.as_ref())
                .map(ScriptToAsm::to_asm),

            is_coinbase,
            sequence: txin.sequence,
            scriptsig: txin.script_sig.clone(),
        }
    }
}

#[derive(Serialize, Clone)]
struct TxOutValue {
    scriptpubkey: Script,
    scriptpubkey_asm: String,
    scriptpubkey_type: String,

    #[serde(skip_serializing_if = "Option::is_none")]
    scriptpubkey_address: Option<String>,
    value: u64,
}

impl TxOutValue {
    fn new(txout: &TxOut, config: &Config) -> Self {
        let value = txout.value.to_sat();
        let is_fee = false;

        let script = &txout.script_pubkey;
        let script_asm = script.to_asm();
        let script_addr = script.to_address_str(config.network_type);

        // TODO should the following something to put inside rust-elements lib?
        let script_type = if is_fee {
            "fee"
        } else if script.is_empty() {
            "empty"
        } else if script.is_op_return() {
            "op_return"
        } else if script.is_p2pk() {
            "p2pk"
        } else if script.is_p2pkh() {
            "p2pkh"
        } else if script.is_p2sh() {
            "p2sh"
        } else if script.is_p2wpkh() {
            "v0_p2wpkh"
        } else if script.is_p2wsh() {
            "v0_p2wsh"
        } else if script.is_p2tr() {
            "v1_p2tr"
        // } else if script.is_provably_unspendable() {
        //     "provably_unspendable"
        } else if script.is_op_return() {
            "op_return"
        } else {
            "unknown"
        };

        TxOutValue {
            scriptpubkey: script.clone(),
            scriptpubkey_asm: script_asm,
            scriptpubkey_address: script_addr,
            scriptpubkey_type: script_type.to_string(),
            value,
        }
    }
}

#[derive(Serialize)]
struct UtxoValue {
    txid: Txid,
    vout: u32,
    status: TransactionStatus,
    value: u64,
}
impl From<Utxo> for UtxoValue {
    fn from(utxo: Utxo) -> Self {
        UtxoValue {
            txid: utxo.txid,
            vout: utxo.vout,
            status: TransactionStatus::from(utxo.confirmed),
            value: utxo.value,
        }
    }
}

#[derive(Serialize)]
struct SpendingValue {
    spent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    txid: Option<Txid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    vin: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<TransactionStatus>,
}
impl From<SpendingInput> for SpendingValue {
    fn from(spend: SpendingInput) -> Self {
        SpendingValue {
            spent: true,
            txid: Some(spend.txid),
            vin: Some(spend.vin),
            status: Some(TransactionStatus::from(spend.confirmed)),
        }
    }
}
impl Default for SpendingValue {
    fn default() -> Self {
        SpendingValue {
            spent: false,
            txid: None,
            vin: None,
            status: None,
        }
    }
}

fn ttl_by_depth(height: Option<usize>, query: &Query) -> u32 {
    height.map_or(TTL_SHORT, |height| {
        if query.chain().best_height() - height >= CONF_FINAL {
            TTL_LONG
        } else {
            TTL_SHORT
        }
    })
}

fn prepare_txs(
    txs: Vec<(Transaction, Option<BlockId>)>,
    query: &Query,
    config: &Config,
) -> Vec<TransactionValue> {
    // let outpoints = txs
    //     .iter()
    //     .flat_map(|(tx, _)| {
    //         tx.input
    //             .iter()
    //             .filter(|txin| has_prevout(txin))
    //             .map(|txin| txin.previous_output)
    //     })
    //     .collect();
    let skip_output = crate::new_index::schema::get_skip_outpoint();
    let outpoints = txs
            .iter()
            .flat_map(|(tx, _)| {
                tx.input
                    .iter()
                    .filter(|txin| has_prevout(txin))
                    .map(|txin| txin.previous_output)
    })
    .filter(|outpoint| *outpoint != skip_output)
    .collect();

    let mut prevouts = query.lookup_txos(outpoints);
    
    // filter out the skip_outpoint
    prevouts.retain(|outpoint, _| *outpoint != crate::new_index::schema::get_skip_outpoint());

    txs.into_iter()
        .map(|(tx, blockid)| TransactionValue::new(tx, blockid, &prevouts, config))
        .collect()
}

#[tokio::main]
async fn run_server(config: Arc<Config>, query: Arc<Query>, rx: oneshot::Receiver<()>) {
    let addr = &config.http_addr;
    let socket_file = &config.http_socket_file;

    let config = Arc::clone(&config);
    let query = Arc::clone(&query);

    let make_service_fn_inn = || {
        let query = Arc::clone(&query);
        let config = Arc::clone(&config);

        async move {
            Ok::<_, hyper::Error>(service_fn(move |req| {
                let query = Arc::clone(&query);
                let config = Arc::clone(&config);

                async move {
                    let method = req.method().clone();
                    let uri = req.uri().clone();
                    let body = hyper::body::to_bytes(req.into_body()).await?;

                    let mut resp = handle_request(method, uri, body, &query, &config)
                        .unwrap_or_else(|err| {
                            warn!("{:?}", err);
                            Response::builder()
                                .status(err.0)
                                .header("Content-Type", "text/plain")
                                .body(Body::from(err.1))
                                .unwrap()
                        });
                    if let Some(ref origins) = config.cors {
                        resp.headers_mut()
                            .insert("Access-Control-Allow-Origin", origins.parse().unwrap());
                    }
                    Ok::<_, hyper::Error>(resp)
                }
            }))
        }
    };

    let server = match socket_file {
        None => {
            info!("REST server running on {}", addr);

            let socket = create_socket(&addr);
            socket.listen(511).expect("setting backlog failed");

            Server::from_tcp(socket.into())
                .expect("Server::from_tcp failed")
                .serve(make_service_fn(move |_| make_service_fn_inn()))
                .with_graceful_shutdown(async {
                    rx.await.ok();
                })
                .await
        }
        Some(path) => {
            if let Ok(meta) = fs::metadata(&path) {
                // Cleanup socket file left by previous execution
                if meta.file_type().is_socket() {
                    fs::remove_file(path).ok();
                }
            }

            info!("REST server running on unix socket {}", path.display());

            Server::bind_unix(path)
                .expect("Server::bind_unix failed")
                .serve(make_service_fn(move |_| make_service_fn_inn()))
                .with_graceful_shutdown(async {
                    rx.await.ok();
                })
                .await
        }
    };

    if let Err(e) = server {
        eprintln!("server error: {}", e);
    }
}

pub fn start(config: Arc<Config>, query: Arc<Query>) -> Handle {
    let (tx, rx) = oneshot::channel::<()>();

    Handle {
        tx,
        thread: thread::spawn(move || {
            run_server(config, query, rx);
        }),
    }
}

pub struct Handle {
    tx: oneshot::Sender<()>,
    thread: thread::JoinHandle<()>,
}

impl Handle {
    pub fn stop(self) {
        self.tx.send(()).expect("failed to send shutdown signal");
        self.thread.join().expect("REST server failed");
    }
}

fn handle_request(
    method: Method,
    uri: hyper::Uri,
    body: hyper::body::Bytes,
    query: &Query,
    config: &Config,
) -> Result<Response<Body>, HttpError> {
    // TODO it looks hyper does not have routing and query parsing :(
    let path: Vec<&str> = uri.path().split('/').skip(1).collect();
    let query_params = match uri.query() {
        Some(value) => form_urlencoded::parse(&value.as_bytes())
            .into_owned()
            .collect::<HashMap<String, String>>(),
        None => HashMap::new(),
    };

    info!("handle {:?} {:?}", method, uri);
    match (
        &method,
        path.get(0),
        path.get(1),
        path.get(2),
        path.get(3),
        path.get(4),
    ) {
        (&Method::GET, Some(&"blocks"), Some(&"tip"), Some(&"hash"), None, None) => http_message(
            StatusCode::OK,
            query.chain().best_hash().to_string(),
            TTL_SHORT,
        ),

        (&Method::GET, Some(&"blocks"), Some(&"tip"), Some(&"height"), None, None) => http_message(
            StatusCode::OK,
            query.chain().best_height().to_string(),
            TTL_SHORT,
        ),

        (&Method::GET, Some(&"blocks"), start_height, None, None, None) => {
            let start_height = start_height.and_then(|height| height.parse::<usize>().ok());
            blocks(&query, start_height)
        }
        (&Method::GET, Some(&"block-height"), Some(height), None, None, None) => {
            let height = height.parse::<usize>()?;
            let header = query
                .chain()
                .header_by_height(height)
                .ok_or_else(|| HttpError::not_found("Block not found".to_string()))?;
            let ttl = ttl_by_depth(Some(height), query);
            http_message(StatusCode::OK, header.hash().to_string(), ttl)
        }
        (&Method::GET, Some(&"block"), Some(hash), None, None, None) => {
            let hash = BlockHash::from_str(hash)?;
            let blockhm = query
                .chain()
                .get_block_with_meta(&hash)
                .ok_or_else(|| HttpError::not_found("Block not found".to_string()))?;
            let block_value = BlockValue::new(blockhm);
            json_response(block_value, TTL_LONG)
        }
        (&Method::GET, Some(&"block"), Some(hash), Some(&"status"), None, None) => {
            let hash = BlockHash::from_str(hash)?;
            let status = query.chain().get_block_status(&hash);
            let ttl = ttl_by_depth(status.height, query);
            json_response(status, ttl)
        }
        (&Method::GET, Some(&"block"), Some(hash), Some(&"txids"), None, None) => {
            let hash = BlockHash::from_str(hash)?;
            let txids = query
                .chain()
                .get_block_txids(&hash)
                .ok_or_else(|| HttpError::not_found("Block not found".to_string()))?;
            json_response(txids, TTL_LONG)
        }
        (&Method::GET, Some(&"block"), Some(hash), Some(&"header"), None, None) => {
            let hash = BlockHash::from_str(hash)?;
            let header = query
                .chain()
                .get_block_header(&hash)
                .ok_or_else(|| HttpError::not_found("Block not found".to_string()))?;

            let header_hex = encode::serialize_hex(&header);
            http_message(StatusCode::OK, header_hex, TTL_LONG)
        }
        (&Method::GET, Some(&"block"), Some(hash), Some(&"raw"), None, None) => {
            let hash = BlockHash::from_str(hash)?;
            let raw = query
                .chain()
                .get_block_raw(&hash)
                .ok_or_else(|| HttpError::not_found("Block not found".to_string()))?;

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/octet-stream")
                .header("Cache-Control", format!("public, max-age={:}", TTL_LONG))
                .body(Body::from(raw))
                .unwrap())
        }
        (&Method::GET, Some(&"block"), Some(hash), Some(&"txid"), Some(index), None) => {
            let hash = BlockHash::from_str(hash)?;
            let index: usize = index.parse()?;
            let txids = query
                .chain()
                .get_block_txids(&hash)
                .ok_or_else(|| HttpError::not_found("Block not found".to_string()))?;
            if index >= txids.len() {
                bail!(HttpError::not_found("tx index out of range".to_string()));
            }
            http_message(StatusCode::OK, txids[index].to_string(), TTL_LONG)
        }
        (&Method::GET, Some(&"block"), Some(hash), Some(&"txs"), start_index, None) => {
            let hash = BlockHash::from_str(hash)?;
            let txids = query
                .chain()
                .get_block_txids(&hash)
                .ok_or_else(|| HttpError::not_found("Block not found".to_string()))?;

            let start_index = start_index
                .map_or(0u32, |el| el.parse().unwrap_or(0))
                .max(0u32) as usize;
            if start_index >= txids.len() {
                bail!(HttpError::not_found("start index out of range".to_string()));
            } else if start_index % CHAIN_TXS_PER_PAGE != 0 {
                bail!(HttpError::from(format!(
                    "start index must be a multipication of {}",
                    CHAIN_TXS_PER_PAGE
                )));
            }

            // blockid_by_hash() only returns the BlockId for non-orphaned blocks,
            // or None for orphaned
            let confirmed_blockid = query.chain().blockid_by_hash(&hash);

            let txs = txids
                .iter()
                .skip(start_index)
                .take(CHAIN_TXS_PER_PAGE)
                .map(|txid| {
                    query
                        .lookup_txn(&txid)
                        .map(|tx| (tx, confirmed_blockid.clone()))
                        .ok_or_else(|| "missing tx".to_string())
                })
                .collect::<Result<Vec<(Transaction, Option<BlockId>)>, _>>()?;

            // XXX orphraned blocks alway get TTL_SHORT
            let ttl = ttl_by_depth(confirmed_blockid.map(|b| b.height), query);

            json_response(prepare_txs(txs, query, config), ttl)
        }
        (&Method::GET, Some(script_type @ &"address"), Some(script_str), None, None, None)
        | (&Method::GET, Some(script_type @ &"scripthash"), Some(script_str), None, None, None) => {
            let script_hash = to_scripthash(script_type, script_str, config.network_type)?;
            let stats = query.stats(&script_hash[..]);
            json_response(
                json!({
                    *script_type: script_str,
                    "chain_stats": stats.0,
                    "mempool_stats": stats.1,
                }),
                TTL_SHORT,
            )
        }
        (
            &Method::GET,
            Some(script_type @ &"address"),
            Some(script_str),
            Some(&"txs"),
            None,
            None,
        )
        | (
            &Method::GET,
            Some(script_type @ &"scripthash"),
            Some(script_str),
            Some(&"txs"),
            None,
            None,
        ) => {
            let script_hash = to_scripthash(script_type, script_str, config.network_type)?;

            let mut txs = vec![];

            txs.extend(
                query
                    .mempool()
                    .history(&script_hash[..], MAX_MEMPOOL_TXS)
                    .into_iter()
                    .map(|tx| (tx, None)),
            );

            txs.extend(
                query
                    .chain()
                    .history(&script_hash[..], None, CHAIN_TXS_PER_PAGE)
                    .into_iter()
                    .map(|(tx, blockid)| (tx, Some(blockid))),
            );

            json_response(prepare_txs(txs, query, config), TTL_SHORT)
        }

        (
            &Method::GET,
            Some(script_type @ &"address"),
            Some(script_str),
            Some(&"txs"),
            Some(&"chain"),
            last_seen_txid,
        )
        | (
            &Method::GET,
            Some(script_type @ &"scripthash"),
            Some(script_str),
            Some(&"txs"),
            Some(&"chain"),
            last_seen_txid,
        ) => {
            let script_hash = to_scripthash(script_type, script_str, config.network_type)?;
            let last_seen_txid = last_seen_txid.and_then(|txid| Txid::from_str(txid).ok());

            let txs = query
                .chain()
                .history(
                    &script_hash[..],
                    last_seen_txid.as_ref(),
                    CHAIN_TXS_PER_PAGE,
                )
                .into_iter()
                .map(|(tx, blockid)| (tx, Some(blockid)))
                .collect();

            json_response(prepare_txs(txs, query, config), TTL_SHORT)
        }
        (
            &Method::GET,
            Some(script_type @ &"address"),
            Some(script_str),
            Some(&"txs"),
            Some(&"mempool"),
            None,
        )
        | (
            &Method::GET,
            Some(script_type @ &"scripthash"),
            Some(script_str),
            Some(&"txs"),
            Some(&"mempool"),
            None,
        ) => {
            let script_hash = to_scripthash(script_type, script_str, config.network_type)?;

            let txs = query
                .mempool()
                .history(&script_hash[..], MAX_MEMPOOL_TXS)
                .into_iter()
                .map(|tx| (tx, None))
                .collect();

            json_response(prepare_txs(txs, query, config), TTL_SHORT)
        }

        (
            &Method::GET,
            Some(script_type @ &"address"),
            Some(script_str),
            Some(&"utxo"),
            None,
            None,
        )
        | (
            &Method::GET,
            Some(script_type @ &"scripthash"),
            Some(script_str),
            Some(&"utxo"),
            None,
            None,
        ) => {
            let script_hash = to_scripthash(script_type, script_str, config.network_type)?;
            let utxos: Vec<UtxoValue> = query
                .utxo(&script_hash[..])?
                .into_iter()
                .map(UtxoValue::from)
                .collect();
            // XXX paging?
            json_response(utxos, TTL_SHORT)
        }
        (&Method::GET, Some(&"address-prefix"), Some(prefix), None, None, None) => {
            if !config.address_search {
                return Err(HttpError::from("address search disabled".to_string()));
            }
            let results = query.chain().address_search(prefix, ADDRESS_SEARCH_LIMIT);
            json_response(results, TTL_SHORT)
        }
        (&Method::GET, Some(&"tx"), Some(hash), None, None, None) => {
            let hash = Txid::from_str(hash)?;
            let tx = query
                .lookup_txn(&hash)
                .ok_or_else(|| HttpError::not_found("Transaction not found".to_string()))?;
            let blockid = query.chain().tx_confirming_block(&hash);
            let ttl = ttl_by_depth(blockid.as_ref().map(|b| b.height), query);

            let tx = prepare_txs(vec![(tx, blockid)], query, config).remove(0);

            json_response(tx, ttl)
        }
        (&Method::GET, Some(&"tx"), Some(hash), Some(out_type @ &"hex"), None, None)
        | (&Method::GET, Some(&"tx"), Some(hash), Some(out_type @ &"raw"), None, None) => {
            let hash = Txid::from_str(hash)?;
            let rawtx = query
                .lookup_raw_txn(&hash)
                .ok_or_else(|| HttpError::not_found("Transaction not found".to_string()))?;

            let (content_type, body) = match *out_type {
                "raw" => ("application/octet-stream", Body::from(rawtx)),
                "hex" => ("text/plain", Body::from(rawtx.to_lower_hex_string())),
                _ => unreachable!(),
            };
            let ttl = ttl_by_depth(query.get_tx_status(&hash).block_height, query);

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", content_type)
                .header("Cache-Control", format!("public, max-age={:}", ttl))
                .body(body)
                .unwrap())
        }
        (&Method::GET, Some(&"tx"), Some(hash), Some(&"status"), None, None) => {
            let hash = Txid::from_str(hash)?;
            let status = query.get_tx_status(&hash);
            let ttl = ttl_by_depth(status.block_height, query);
            json_response(status, ttl)
        }

        (&Method::GET, Some(&"tx"), Some(hash), Some(&"merkle-proof"), None, None) => {
            let hash = Txid::from_str(hash)?;
            let blockid = query.chain().tx_confirming_block(&hash).ok_or_else(|| {
                HttpError::not_found("Transaction not found or is unconfirmed".to_string())
            })?;
            let (merkle, pos) =
                electrum_merkle::get_tx_merkle_proof(query.chain(), &hash, &blockid.hash)?;
            let merkle: Vec<String> = merkle.into_iter().map(|txid| txid.to_string()).collect();
            let ttl = ttl_by_depth(Some(blockid.height), query);
            json_response(
                json!({ "block_height": blockid.height, "merkle": merkle, "pos": pos }),
                ttl,
            )
        }

        (&Method::GET, Some(&"tx"), Some(hash), Some(&"merkleblock-proof"), None, None) => {
            let hash = Txid::from_str(hash)?;

            let merkleblock = query.chain().get_merkleblock_proof(&hash).ok_or_else(|| {
                HttpError::not_found("Transaction not found or is unconfirmed".to_string())
            })?;

            let height = query
                .chain()
                .height_by_hash(&merkleblock.header.block_hash());

            http_message(
                StatusCode::OK,
                encode::serialize_hex(&merkleblock),
                ttl_by_depth(height, query),
            )
        }
        (&Method::GET, Some(&"tx"), Some(hash), Some(&"outspend"), Some(index), None) => {
            let hash = Txid::from_str(hash)?;
            let outpoint = OutPoint {
                txid: hash,
                vout: index.parse::<u32>()?,
            };
            let spend = query
                .lookup_spend(&outpoint)
                .map_or_else(SpendingValue::default, SpendingValue::from);
            let ttl = ttl_by_depth(
                spend
                    .status
                    .as_ref()
                    .and_then(|ref status| status.block_height),
                query,
            );
            json_response(spend, ttl)
        }
        (&Method::GET, Some(&"tx"), Some(hash), Some(&"outspends"), None, None) => {
            let hash = Txid::from_str(hash)?;
            let tx = query
                .lookup_txn(&hash)
                .ok_or_else(|| HttpError::not_found("Transaction not found".to_string()))?;
            let spends: Vec<SpendingValue> = query
                .lookup_tx_spends(tx)
                .into_iter()
                .map(|spend| spend.map_or_else(SpendingValue::default, SpendingValue::from))
                .collect();
            // @TODO long ttl if all outputs are either spent long ago or unspendable
            json_response(spends, TTL_SHORT)
        }
        (&Method::GET, Some(&"broadcast"), None, None, None, None)
        | (&Method::POST, Some(&"tx"), None, None, None, None) => {
            // accept both POST and GET for backward compatibility.
            // GET will eventually be removed in favor of POST.
            let txhex = match method {
                Method::POST => String::from_utf8(body.to_vec())?,
                Method::GET => query_params
                    .get("tx")
                    .cloned()
                    .ok_or_else(|| HttpError::from("Missing tx".to_string()))?,
                _ => return http_message(StatusCode::METHOD_NOT_ALLOWED, "Invalid method", 0),
            };
            let txid = query
                .broadcast_raw(&txhex)
                .map_err(|err| HttpError::from(err.description().to_string()))?;
            http_message(StatusCode::OK, txid.to_string(), 0)
        }

        (&Method::GET, Some(&"mempool"), None, None, None, None) => {
            json_response(query.mempool().backlog_stats(), TTL_SHORT)
        }
        (&Method::GET, Some(&"mempool"), Some(&"txids"), None, None, None) => {
            json_response(query.mempool().txids(), TTL_SHORT)
        }
        (&Method::GET, Some(&"mempool"), Some(&"recent"), None, None, None) => {
            let mempool = query.mempool();
            let recent = mempool.recent_txs_overview();
            json_response(recent, TTL_MEMPOOL_RECENT)
        }

        (&Method::GET, Some(&"fee-estimates"), None, None, None, None) => {
            json_response(query.estimate_fee_map(), TTL_SHORT)
        }

        _ => Err(HttpError::not_found(format!(
            "endpoint does not exist {:?}",
            uri.path()
        ))),
    }
}

fn http_message<T>(status: StatusCode, message: T, ttl: u32) -> Result<Response<Body>, HttpError>
where
    T: Into<Body>,
{
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .header("Cache-Control", format!("public, max-age={:}", ttl))
        .body(message.into())
        .unwrap())
}

fn json_response<T: Serialize>(value: T, ttl: u32) -> Result<Response<Body>, HttpError> {
    let value = serde_json::to_string(&value)?;
    Ok(Response::builder()
        .header("Content-Type", "application/json")
        .header("Cache-Control", format!("public, max-age={:}", ttl))
        .body(Body::from(value))
        .unwrap())
}

fn blocks(query: &Query, start_height: Option<usize>) -> Result<Response<Body>, HttpError> {
    let mut values = Vec::new();
    let mut current_hash = match start_height {
        Some(height) => *query
            .chain()
            .header_by_height(height)
            .ok_or_else(|| HttpError::not_found("Block not found".to_string()))?
            .hash(),
        None => query.chain().best_hash(),
    };

    let zero = [0u8; 32];
    for _ in 0..BLOCK_LIMIT {
        let blockhm = query
            .chain()
            .get_block_with_meta(&current_hash)
            .ok_or_else(|| HttpError::not_found("Block not found".to_string()))?;
        current_hash = blockhm.header_entry.header().prev_blockhash;

        #[allow(unused_mut)]
        let mut value = BlockValue::new(blockhm);
        values.push(value);

        if current_hash[..] == zero[..] {
            break;
        }
    }
    json_response(values, TTL_SHORT)
}

fn to_scripthash(
    script_type: &str,
    script_str: &str,
    network: Network,
) -> Result<FullHash, HttpError> {
    match script_type {
        "address" => address_to_scripthash(script_str, network),
        "scripthash" => parse_scripthash(script_str),
        _ => bail!("Invalid script type".to_string()),
    }
}

fn address_to_scripthash(addr: &str, network: Network) -> Result<FullHash, HttpError> {
    let addr = address::Address::from_str(addr)?;
    let is_expected_net = addr.is_valid_for_network(network.into());
    if !is_expected_net {
        bail!(HttpError::from("Address on invalid network".to_string()))
    }

    let addr = addr.assume_checked();

    Ok(compute_script_hash(&addr.script_pubkey()))
}

fn parse_scripthash(scripthash: &str) -> Result<FullHash, HttpError> {
    FullHash::from_hex(scripthash).map_err(|_| HttpError::from("Invalid scripthash".to_string()))
}

#[derive(Debug)]
struct HttpError(StatusCode, String);

impl HttpError {
    fn not_found(msg: String) -> Self {
        HttpError(StatusCode::NOT_FOUND, msg)
    }
}

impl From<String> for HttpError {
    fn from(msg: String) -> Self {
        HttpError(StatusCode::BAD_REQUEST, msg)
    }
}
impl From<ParseIntError> for HttpError {
    fn from(_e: ParseIntError) -> Self {
        //HttpError::from(e.description().to_string())
        HttpError::from("Invalid number".to_string())
    }
}
impl From<HashError> for HttpError {
    fn from(_e: HashError) -> Self {
        //HttpError::from(e.description().to_string())
        HttpError::from("Invalid hash string".to_string())
    }
}
impl From<hex::HexToBytesError> for HttpError {
    fn from(_e: hex::HexToBytesError) -> Self {
        //HttpError::from(e.description().to_string())
        HttpError::from("Invalid hex string".to_string())
    }
}
impl From<hex::HexToArrayError> for HttpError {
    fn from(_e: hex::HexToArrayError) -> Self {
        //HttpError::from(e.description().to_string())
        HttpError::from("Invalid hex string".to_string())
    }
}
impl From<satsnet::hex::HexToArrayError> for HttpError {
    fn from(_e: satsnet::hex::HexToArrayError) -> Self {
        //HttpError::from(e.description().to_string())
        HttpError::from("Invalid Bitcoin address".to_string())
    }
}
impl From<errors::Error> for HttpError {
    fn from(e: errors::Error) -> Self {
        warn!("errors::Error: {:?}", e);
        match e.description().to_string().as_ref() {
            "getblock RPC error: {\"code\":-5,\"message\":\"Block not found\"}" => {
                HttpError::not_found("Block not found".to_string())
            }
            _ => HttpError::from(e.to_string()),
        }
    }
}
impl From<serde_json::Error> for HttpError {
    fn from(e: serde_json::Error) -> Self {
        HttpError::from(e.to_string())
    }
}
impl From<encode::Error> for HttpError {
    fn from(e: encode::Error) -> Self {
        HttpError::from(e.to_string())
    }
}
impl From<std::string::FromUtf8Error> for HttpError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        HttpError::from(e.to_string())
    }
}

impl From<address::ParseError> for HttpError {
    fn from(e: address::ParseError) -> Self {
        HttpError::from(e.to_string())
    }
}
