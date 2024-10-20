## Installation

Install [latest Rust](https://rustup.rs/) (1.31+),
[latest satsnet Core](https://github.com/sat20-labs/satoshinet) (0.16+)


Also, install the following packages (on Debian):
```bash
$ sudo apt update
$ sudo apt install clang cmake  # for building 'rust-rocksdb'
```

## Build
```bash
$ cargo build --release
```


## satsnet configuration

Allow satsnet daemon to sync before starting Electrum server:
```bash
$ satsnet_btcd_l --homedir /data/satsnet/data --txindex
```

satsnet Configure `-rpcuser=USER` and `-rpcpassword=PASSWORD` for authentication, please use `--cookie="USER:PASSWORD"` command-line flag.

## Usage

First index sync should take ~1.5 hours:
```bash
$ cargo run --release -- -vvv --timestamp \
    --cookie q17AIoqBJSEhW7djqjn0nTsZcz4=:nnlkAZn58bqsyYwVtHIajZ16cj8= \
    --db-dir ./db --network testnet4 \
    --daemon-rpc-addr 192.168.10.104:14827 --daemon-cert-path ./satsnet-rpc.cert \
    --electrum-rpc-addr 0.0.0.0:50001 \
    --http-addr 0.0.0.0:3000 \
    --jsonrpc-import --cors "*" \
    --address-search --index-unspendables \
    --utxos-limit 5000 --electrum-txs-limit 5000
2018-08-17T18:27:42 - INFO - NetworkInfo { version: 179900 }
2018-08-17T18:27:42 - INFO - BlockchainInfo { chain: "main", blocks: 537204, headers: 537204, bestblockhash: "0000000000000000002956768ca9421a8ddf4e53b1d81e429bd0125a383e3636", pruned: false }
2018-08-17T18:27:42 - DEBUG - opening DB at "./db/testnet4"
2018-08-17T18:27:42 - DEBUG - full compaction marker: None
2018-08-17T18:27:42 - INFO - listing block files at "/home/user/.bitcoin/blocks/blk*.dat"
2018-08-17T18:27:42 - INFO - indexing 1348 blk*.dat files
2018-08-17T18:27:42 - DEBUG - found 0 indexed blocks
2018-08-17T18:27:55 - DEBUG - applying 537205 new headers from height 0
2018-08-17T19:31:01 - DEBUG - no more blocks to index
2018-08-17T19:31:03 - DEBUG - no more blocks to index
2018-08-17T19:31:03 - DEBUG - last indexed block: best=0000000000000000002956768ca9421a8ddf4e53b1d81e429bd0125a383e3636 height=537204 @ 2018-08-17T15:24:02Z
2018-08-17T19:31:05 - DEBUG - opening DB at "./db/testnet4"
2018-08-17T19:31:06 - INFO - starting full compaction
2018-08-17T19:58:19 - INFO - finished full compaction
2018-08-17T19:58:19 - INFO - enabling auto-compactions
2018-08-17T19:58:19 - DEBUG - opening DB at "./db/mainnet"
2018-08-17T19:58:26 - DEBUG - applying 537205 new headers from height 0
2018-08-17T19:58:27 - DEBUG - downloading new block headers (537205 already indexed) from 000000000000000000150d26fcc38b8c3b71ae074028d1d50949ef5aa429da00
2018-08-17T19:58:27 - INFO - best=000000000000000000150d26fcc38b8c3b71ae074028d1d50949ef5aa429da00 height=537218 @ 2018-08-17T16:57:50Z (14 left to index)
2018-08-17T19:58:28 - DEBUG - applying 14 new headers from height 537205
2018-08-17T19:58:29 - INFO - RPC server running on 127.0.0.1:50001
```

The index database is stored here:
```bash
$ du db/
38G db/mainnet/
```

## Electrum client
```bash
# Connect only to the local server, for better privacy
$ ./scripts/local-electrum.bash
+ ADDR=127.0.0.1
+ PORT=50001
+ PROTOCOL=t
+ electrum --oneserver --server=127.0.0.1:50001:t
<snip>
```

In order to use a secure connection, TLS-terminating proxy (e.g. [hitch](https://github.com/varnish/hitch)) is recommended:
```bash
$ hitch --backend=[127.0.0.1]:50001 --frontent=[127.0.0.1]:50002 pem_file
$ electrum --oneserver --server=127.0.0.1:50002:s
```

## Docker
```bash
$ docker build -t electrs-app .
$ docker run --network host \
             --volume /home/roman/.bitcoin:/home/user/.bitcoin:ro \
             --volume $PWD:/home/user \
             --rm -i -t electrs-app
```

## Monitoring

Indexing and serving metrics are exported via [Prometheus](https://github.com/pingcap/rust-prometheus):

```bash
$ sudo apt install prometheus
$ echo "
scrape_configs:
  - job_name: electrs
    static_configs:
    - targets: ['localhost:4224']
" | sudo tee -a /etc/prometheus/prometheus.yml
$ sudo systemctl restart prometheus
$ firefox 'http://localhost:9090/graph?g0.range_input=1h&g0.expr=index_height&g0.tab=0'
```
