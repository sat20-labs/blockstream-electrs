# Esplora - Electrs backend API

A block chain index engine and HTTP API written in Rust based on [romanz/electrs](https://github.com/romanz/electrs).

Used as the backend for the [Esplora block explorer](https://github.com/sat20-labs/satsnet-esplora) powering [sat20.org](https://sat20.org/).

API documentation [is available here](https://github.com/sat20-labs/satsnet-esplora/blob/master/API.md).

Documentation for the database schema and indexing process [is available here](doc/schema.md).

### Installing & indexing

Install Rust, Satsnet then

```bash
$ git clone https://github.com/sat20-labs/blockstream-electrs && cd blockstream-electrs
$ git checkout satsnet
$ cargo run --release --bin electrs -- -vvvv \
    --cookie q17AIoqBJSEhW7djqjn0nTsZcz4=:nnlkAZn58bqsyYwVtHIajZ16cj8= \
    --db-dir ./data --network testnet4 \
    --daemon-rpc-addr 192.168.10.104:14827 --daemon-cert-path ./satsnet-rpc.cert \
    --electrum-rpc-addr 0.0.0.0:60301 \
    --http-addr 0.0.0.0:3004 \
    --jsonrpc-import --cors "*" \
    --address-search --index-unspendables \
    --utxos-limit 5000 --electrum-txs-limit 5000
```

See [electrs's original documentation](https://github.com/romanz/electrs/blob/master/doc/usage.md) for more detailed instructions.
Note that our indexes are incompatible with electrs's and has to be created separately.

The indexes require 610GB of storage after running compaction (as of June 2020), but you'll need to have
free space of about double that available during the index compaction process.
Creating the indexes should take a few hours on a beefy machine with SSD.

To deploy with Docker, follow the [instructions here](https://github.com/sat20-labs/satsnet-esplora#how-to-build-the-docker-image).

### Notable changes from Electrs:

- HTTP REST API in addition to the Electrum JSON-RPC protocol, with extended transaction information
  (previous outputs, spending transactions, script asm and more).

- Extended indexes and database storage for improved performance under high load:

  - A full transaction store mapping txids to raw transactions is kept in the database under the prefix `t`.
  - An index of all spendable transaction outputs is kept under the prefix `O`.
  - An index of all addresses (encoded as string) is kept under the prefix `a` to enable by-prefix address search.
  - A map of blockhash to txids is kept in the database under the prefix `X`.
  - Block stats metadata (number of transactions, size and weight) is kept in the database under the prefix `M`.

  With these new indexes, satsnet is no longer queried to serve user requests and is only polled
  periodically for new blocks and for syncing the mempool.

### CLI options

In addition to electrs's original configuration options, a few new options are also available:

- `--http-addr <addr:port>` - HTTP server address/port to listen on (default: `127.0.0.1:3000`).
- `--lightmode` - enable light mode (see above)
- `--cors <origins>` - origins allowed to make cross-site request (optional, defaults to none).
- `--address-search` - enables the by-prefix address search index.
- `--index-unspendables` - enables indexing of provably unspendable outputs.
- `--utxos-limit <num>` - maximum number of utxos to return per address.
- `--electrum-txs-limit <num>` - maximum number of txs to return per address in the electrum server (does not apply for the http api).
- `--electrum-banner <text>` - welcome banner text for electrum server.

See `$ cargo run --release --bin electrs -- --help` for the full list of options.

## License

MIT
