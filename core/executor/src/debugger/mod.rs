#![allow(dead_code)]

mod create2;
mod uniswap2;

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use evm::tracing::{Event, EventListener};

use common_config_parser::parse_file;
use common_crypto::{PrivateKey, Secp256k1RecoverablePrivateKey, Signature};
use protocol::codec::{hex_decode, ProtocolCodec};
use protocol::traits::{Backend, Executor};
use protocol::trie::Trie as _;
use protocol::types::{
    Account, Eip1559Transaction, ExecResp, ExecutorContext, Hash, Hasher, RichBlock,
    SignedTransaction, TxResp, UnsignedTransaction, UnverifiedTransaction, H160, H256,
    MAX_BLOCK_GAS_LIMIT, NIL_DATA, RLP_NULL, U256,
};

use core_db::RocksAdapter;
use core_storage::ImplStorage;

use crate::adapter::{AxonExecutorApplyAdapter, MPTTrie};
use crate::{AxonExecutor, RocksTrieDB};

const GENESIS_PATH: &str = "../../tests/data/genesis.json";
pub const CROSSCHAIN_CONTRACT_ADDRESS: H160 = H160([
    180, 132, 253, 72, 14, 89, 134, 33, 99, 143, 56, 15, 64, 70, 151, 205, 159, 88, 176, 248,
]);

pub struct EvmDebugger {
    state_root: H256,
    storage:    Arc<ImplStorage<RocksAdapter>>,
    trie_db:    Arc<RocksTrieDB>,
}

impl EvmDebugger {
    pub fn new(distribute_addresses: Vec<H160>, distribute_amount: U256, db_path: &str) -> Self {
        let mut db_data_path = db_path.to_string();
        db_data_path.push_str("/data");
        let _ = std::fs::create_dir_all(&db_data_path);
        let rocks_adapter = Arc::new(RocksAdapter::new(db_data_path, Default::default()).unwrap());

        let mut db_state_path = db_path.to_string();
        db_state_path.push_str("/state");
        let _ = std::fs::create_dir_all(&db_state_path);
        let inner_db = rocks_adapter.inner_db();
        let trie = Arc::new(RocksTrieDB::new_evm(inner_db, 1000));

        let mut mpt = MPTTrie::new(Arc::clone(&trie));

        for distribute_address in distribute_addresses.into_iter() {
            let distribute_account = Account {
                nonce:        U256::zero(),
                balance:      distribute_amount,
                storage_root: RLP_NULL,
                code_hash:    NIL_DATA,
            };

            mpt.insert(
                distribute_address.as_bytes().to_vec(),
                distribute_account.encode().unwrap().to_vec(),
            )
            .unwrap();
        }

        EvmDebugger {
            state_root: mpt.commit().unwrap(),
            storage:    Arc::new(ImplStorage::new(rocks_adapter, 10)),
            trie_db:    trie,
        }
    }

    pub fn init_genesis(&mut self) {
        let genesis: RichBlock = parse_file(GENESIS_PATH, true).unwrap();
        self.exec(0, genesis.txs);
    }

    pub fn exec(&mut self, number: u64, txs: Vec<SignedTransaction>) -> ExecResp {
        let mut backend = self.backend(number);
        let res = AxonExecutor.test_exec(&mut backend, &txs, &[]);
        self.state_root = res.state_root;
        res
    }

    #[allow(dead_code)]
    pub fn call(
        &self,
        number: u64,
        from: Option<H160>,
        to: Option<H160>,
        value: U256,
        data: Vec<u8>,
    ) -> TxResp {
        let backend = self.backend(number);
        AxonExecutor.call(&backend, MAX_BLOCK_GAS_LIMIT, from, to, value, data)
    }

    fn backend(
        &self,
        number: u64,
    ) -> AxonExecutorApplyAdapter<ImplStorage<RocksAdapter>, RocksTrieDB> {
        let exec_ctx = ExecutorContext {
            block_number:           number.into(),
            block_coinbase:         H160::random(),
            block_timestamp:        time_now().into(),
            chain_id:               5u64.into(),
            origin:                 H160::random(),
            gas_price:              1u64.into(),
            block_gas_limit:        4294967295000u64.into(),
            block_base_fee_per_gas: 1337u64.into(),
            extra_data:             Default::default(),
        };

        AxonExecutorApplyAdapter::from_root(
            self.state_root,
            Arc::clone(&self.trie_db),
            Arc::clone(&self.storage),
            exec_ctx,
        )
        .unwrap()
    }

    fn nonce(&self, addr: H160) -> U256 {
        self.backend(0).basic(addr).nonce
    }
}

#[derive(Default)]
pub struct EvmListener;

impl EventListener for EvmListener {
    fn event(&mut self, event: Event) {
        println!("EVM event {:?}", event);
    }
}

pub fn mock_efficient_signed_tx(tx: Eip1559Transaction, private_key: &str) -> SignedTransaction {
    let priv_key =
        Secp256k1RecoverablePrivateKey::try_from(hex_decode(private_key).unwrap().as_ref())
            .expect("Invalid secp private key");

    let tx = UnsignedTransaction::Eip1559(tx);
    let signature = priv_key.sign_message(
        &Hasher::digest(tx.encode(Some(5u64), None))
            .as_bytes()
            .try_into()
            .unwrap(),
    );

    let utx = UnverifiedTransaction {
        unsigned:  tx,
        hash:      Hash::default(),
        chain_id:  Some(5u64),
        signature: Some(signature.to_bytes().into()),
    }
    .calc_hash();

    SignedTransaction::from_unverified(utx, None).unwrap()
}

pub fn mock_signed_tx(tx: Eip1559Transaction, sender: H160) -> SignedTransaction {
    let utx = UnverifiedTransaction {
        unsigned:  UnsignedTransaction::Eip1559(tx),
        hash:      Hash::default(),
        chain_id:  Some(5u64),
        signature: None,
    };

    SignedTransaction {
        transaction: utx,
        sender,
        public: None,
    }
}

pub fn clear_data(db_path: &str) {
    std::fs::remove_dir_all(db_path).unwrap()
}

fn time_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
