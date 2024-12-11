// use regular Bitcoin data structures
pub use bitcoin::{
    address, blockdata::block::Header as BlockHeader, blockdata::script, consensus::deserialize,
    hash_types::TxMerkleNode, Address, Block, BlockHash, OutPoint, ScriptBuf as Script, Sequence,
    Transaction, TxIn, TxOut, Txid,
};


use bitcoin::blockdata::constants::genesis_block;
pub use bitcoin::network::Network as BNetwork;

pub type Value = u64;

#[derive(Debug, Copy, Clone, PartialEq, Hash, Serialize, Ord, PartialOrd, Eq)]
pub enum Network {
    Bitcoin,
    Testnet,
    Testnet4,
    Regtest,
    Signet,
    Satsnet,
    Satstestnet,
}

impl Network {
    pub fn magic(self) -> u32 {
        u32::from_le_bytes(BNetwork::from(self).magic().to_bytes())
    }

    pub fn is_regtest(self) -> bool {
        match self {
            Network::Regtest => true,
            _ => false,
        }
    }

    pub fn names() -> Vec<String> {
        return vec![
            "mainnet".to_string(),
            "testnet".to_string(),
            "regtest".to_string(),
            "signet".to_string(),
            "satsnet".to_string(),
            "satstestnet".to_string(),
        ];
    }
}

pub fn genesis_hash(network: Network) -> BlockHash {
    return bitcoin_genesis_hash(network.into());
}

pub fn bitcoin_genesis_hash(network: BNetwork) -> bitcoin::BlockHash {
    lazy_static! {
        static ref BITCOIN_GENESIS: bitcoin::BlockHash =
            genesis_block(BNetwork::Bitcoin).block_hash();
        static ref TESTNET_GENESIS: bitcoin::BlockHash =
            genesis_block(BNetwork::Testnet).block_hash();
        static ref TESTNET4_GENESIS: bitcoin::BlockHash =
            genesis_block(BNetwork::Testnet4).block_hash();
        static ref REGTEST_GENESIS: bitcoin::BlockHash =
            genesis_block(BNetwork::Regtest).block_hash();
        static ref SIGNET_GENESIS: bitcoin::BlockHash =
            genesis_block(BNetwork::Signet).block_hash();
        static ref SATSNET_GENESIS: bitcoin::BlockHash =
            genesis_block(BNetwork::Satsnet).block_hash();
        static ref SATSTESTNET_GENESIS: bitcoin::BlockHash =
            genesis_block(BNetwork::Satstestnet).block_hash();
    }

    match network {
        BNetwork::Bitcoin => *BITCOIN_GENESIS,
        BNetwork::Testnet => *TESTNET_GENESIS,
        BNetwork::Testnet4 => *TESTNET4_GENESIS,
        BNetwork::Regtest => *REGTEST_GENESIS,
        BNetwork::Signet => *SIGNET_GENESIS,
        BNetwork::Satsnet => *SATSNET_GENESIS,
        BNetwork::Satstestnet => *SATSTESTNET_GENESIS,
        _ => panic!("unknown network {:?}", network),
    }
}

impl From<&str> for Network {
    fn from(network_name: &str) -> Self {
        match network_name {
            "mainnet" => Network::Bitcoin,
            "testnet" => Network::Testnet,
            "testnet4" => Network::Testnet4,
            "regtest" => Network::Regtest,
            "signet" => Network::Signet,
            "satsnet" => Network::Satsnet,
            "satstestnet" => Network::Satstestnet,
            _ => panic!("unsupported Bitcoin network: {:?}", network_name),
        }
    }
}

impl From<Network> for BNetwork {
    fn from(network: Network) -> Self {
        match network {
            Network::Bitcoin => BNetwork::Bitcoin,
            Network::Testnet => BNetwork::Testnet,
            Network::Testnet4 => BNetwork::Testnet4,
            Network::Regtest => BNetwork::Regtest,
            Network::Signet => BNetwork::Signet,
            Network::Satsnet => BNetwork::Satsnet,
            Network::Satstestnet => BNetwork::Satstestnet,
        }
    }
}

impl From<BNetwork> for Network {
    fn from(network: BNetwork) -> Self {
        match network {
            BNetwork::Bitcoin => Network::Bitcoin,
            BNetwork::Testnet => Network::Testnet,
            BNetwork::Testnet4 => Network::Testnet4,
            BNetwork::Regtest => Network::Regtest,
            BNetwork::Signet => Network::Signet,
            BNetwork::Satsnet => Network::Satsnet,
            BNetwork::Satstestnet => Network::Satstestnet,
            _ => panic!("unknown network {:?}", network),
        }
    }
}
