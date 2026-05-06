use std::{
    collections::{BTreeMap, VecDeque, hash_map::Entry},
    sync::RwLock,
};

use rustc_hash::{FxHashMap, FxHashSet};

use crate::{
    constants::{
        TX_ACCESS_LIST_ADDRESS_GAS, TX_ACCESS_LIST_STORAGE_KEY_GAS, TX_CREATE_GAS_COST,
        TX_DATA_NON_ZERO_GAS, TX_DATA_NON_ZERO_GAS_EIP2028, TX_DATA_ZERO_GAS_COST, TX_GAS_COST,
        TX_INIT_CODE_WORD_GAS_COST,
    },
    error::MempoolError,
};
use ethrex_common::{
    Address, H160, H256, U256,
    types::{
        BlobTuple, BlobsBundle, BlockHeader, ChainConfig, MempoolTransaction, Transaction, TxType,
        kzg_commitment_to_versioned_hash,
    },
};
use ethrex_storage::error::StoreError;
use tracing::warn;

const DEFAULT_REPLACEMENT_PRICE_BUMP: u128 = 10;
const BLOB_REPLACEMENT_PRICE_BUMP: u128 = 100;

#[derive(Debug, Default)]
struct MempoolInner {
    broadcast_pool: FxHashSet<H256>,
    transaction_pool: FxHashMap<H256, MempoolTransaction>,
    blobs_bundle_pool: FxHashMap<H256, BlobsBundle>,
    /// Transaction hashes that have been requested via GetPooledTransactions
    /// but whose responses haven't arrived yet. Used to avoid sending duplicate
    /// requests when multiple peers announce the same transaction.
    in_flight_txs: FxHashSet<H256>,
    /// Maps blob versioned hashes to transaction hashes that include them and a position inside
    /// blob bundle where blob and its adjacent data is available.
    blobs_bundle_by_versioned_hash: FxHashMap<H256, FxHashMap<H256, usize>>,
    txs_by_sender_nonce: BTreeMap<(H160, u64), H256>,
    txs_order: VecDeque<H256>,
    max_mempool_size: usize,
    // Max number of transactions to let the mempool order queue grow before pruning it
    mempool_prune_threshold: usize,
}

impl MempoolInner {
    fn new(max_mempool_size: usize) -> Self {
        MempoolInner {
            txs_order: VecDeque::with_capacity(max_mempool_size * 2),
            transaction_pool: FxHashMap::with_capacity_and_hasher(
                max_mempool_size,
                Default::default(),
            ),
            max_mempool_size,
            mempool_prune_threshold: max_mempool_size + max_mempool_size / 2,
            ..Default::default()
        }
    }

    /// Remove a transaction from the pool with the transaction pool lock already taken
    fn remove_transaction_with_lock(&mut self, hash: &H256) -> Result<(), StoreError> {
        let Some(tx) = self.transaction_pool.remove(hash) else {
            return Ok(());
        };
        if matches!(tx.tx_type(), TxType::EIP4844) {
            self.remove_blob_bundle(hash);
        }

        self.txs_by_sender_nonce.remove(&(tx.sender(), tx.nonce()));
        self.broadcast_pool.remove(hash);

        Ok(())
    }

    /// Remove a blobs bundle from the pool
    pub fn remove_blob_bundle(&mut self, hash: &H256) {
        let Some(h) = self.blobs_bundle_pool.remove(hash) else {
            return;
        };

        for commitment in &h.commitments {
            let versioned_hash = kzg_commitment_to_versioned_hash(commitment);
            if let Entry::Occupied(mut entry) =
                self.blobs_bundle_by_versioned_hash.entry(versioned_hash)
            {
                let txn_to_bundle = entry.get_mut();
                txn_to_bundle.remove(hash);
                if txn_to_bundle.is_empty() {
                    entry.remove();
                }
            }
        }
    }

    /// Remove the oldest transaction in the pool
    fn remove_oldest_transaction(&mut self) -> Result<(), StoreError> {
        // Remove elements from the order queue until one is present in the pool
        while self.transaction_pool.len() >= self.max_mempool_size {
            if let Some(oldest_hash) = self.txs_order.pop_front() {
                self.remove_transaction_with_lock(&oldest_hash)?;
            } else {
                warn!(
                    "Mempool is full but there are no transactions to remove, this should not happen and will make the mempool grow indefinitely"
                );
                break;
            }
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct Mempool {
    inner: RwLock<MempoolInner>,
    /// Signaled on transaction and blobs bundle insertions so payload
    /// builders can await new work instead of busy-looping.
    tx_added: tokio::sync::Notify,
}

impl Mempool {
    pub fn new(max_mempool_size: usize) -> Self {
        Mempool {
            inner: RwLock::new(MempoolInner::new(max_mempool_size)),
            tx_added: tokio::sync::Notify::new(),
        }
    }

    pub(crate) fn tx_added(&self) -> &tokio::sync::Notify {
        &self.tx_added
    }

    fn write(&self) -> Result<std::sync::RwLockWriteGuard<'_, MempoolInner>, StoreError> {
        self.inner
            .write()
            .map_err(|error| StoreError::MempoolWriteLock(error.to_string()))
    }

    fn read(&self) -> Result<std::sync::RwLockReadGuard<'_, MempoolInner>, StoreError> {
        self.inner
            .read()
            .map_err(|error| StoreError::MempoolReadLock(error.to_string()))
    }

    /// Add transaction to the pool without doing validity checks
    pub fn add_transaction(
        &self,
        hash: H256,
        sender: Address,
        transaction: MempoolTransaction,
    ) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        // Prune the order queue if it has grown too much
        if inner.txs_order.len() > inner.mempool_prune_threshold {
            // NOTE: we do this to avoid borrow checker errors
            let txpool = core::mem::take(&mut inner.transaction_pool);
            inner.txs_order.retain(|tx| txpool.contains_key(tx));
            inner.transaction_pool = txpool;
        }
        if inner.transaction_pool.len() >= inner.max_mempool_size {
            inner.remove_oldest_transaction()?;
        }
        inner.txs_order.push_back(hash);
        inner
            .txs_by_sender_nonce
            .insert((sender, transaction.nonce()), hash);
        inner.transaction_pool.insert(hash, transaction);
        inner.broadcast_pool.insert(hash);
        // Drop the write lock before notifying to avoid holding it while waking waiters
        drop(inner);
        self.tx_added.notify_waiters();

        Ok(())
    }

    pub fn get_txs_for_broadcast(&self) -> Result<Vec<MempoolTransaction>, StoreError> {
        let inner = self.read()?;
        let txs = inner
            .transaction_pool
            .iter()
            .filter_map(|(hash, tx)| {
                if !inner.broadcast_pool.contains(hash) {
                    None
                } else {
                    Some(tx.clone())
                }
            })
            .collect::<Vec<_>>();
        Ok(txs)
    }

    pub fn remove_broadcasted_txs(&self, hashes: &[H256]) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        for hash in hashes {
            inner.broadcast_pool.remove(hash);
        }
        Ok(())
    }

    /// Add a blobs bundle to the pool by its blob transaction hash
    pub fn add_blobs_bundle(
        &self,
        tx_hash: H256,
        blobs_bundle: BlobsBundle,
    ) -> Result<(), StoreError> {
        let mut mempool = self.write()?;
        for (i, c) in blobs_bundle.commitments.iter().enumerate() {
            let versioned_hash = kzg_commitment_to_versioned_hash(c);
            mempool
                .blobs_bundle_by_versioned_hash
                .entry(versioned_hash)
                .or_default()
                .insert(tx_hash, i);
        }
        mempool.blobs_bundle_pool.insert(tx_hash, blobs_bundle);
        Ok(())
    }

    /// Get a blobs bundle to the pool given its blob transaction hash
    pub fn get_blobs_bundle(&self, tx_hash: H256) -> Result<Option<BlobsBundle>, StoreError> {
        Ok(self.read()?.blobs_bundle_pool.get(&tx_hash).cloned())
    }

    /// Remove a transaction from the pool
    pub fn remove_transaction(&self, hash: &H256) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        inner.remove_transaction_with_lock(hash)?;
        Ok(())
    }

    /// Remove every pending transaction and blob bundle from the mempool.
    pub fn clear(&self) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        inner.broadcast_pool.clear();
        inner.transaction_pool.clear();
        inner.blobs_bundle_pool.clear();
        inner.in_flight_txs.clear();
        inner.blobs_bundle_by_versioned_hash.clear();
        inner.txs_by_sender_nonce.clear();
        inner.txs_order.clear();
        drop(inner);
        self.tx_added.notify_waiters();
        Ok(())
    }

    /// Applies the filter and returns a set of suitable transactions from the mempool.
    /// These transactions will be grouped by sender and sorted by nonce
    pub fn filter_transactions(
        &self,
        filter: &PendingTxFilter,
    ) -> Result<FxHashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        self.filter_transactions_with_insertion_order(filter)
            .map(|(txs, _)| txs)
    }

    /// Applies the filter and returns suitable transactions grouped by sender, together with
    /// their mempool insertion order.
    pub fn filter_transactions_with_insertion_order(
        &self,
        filter: &PendingTxFilter,
    ) -> Result<
        (
            FxHashMap<Address, Vec<MempoolTransaction>>,
            FxHashMap<H256, usize>,
        ),
        StoreError,
    > {
        self.filter_transactions_with_order(&|tx| pending_tx_matches_filter(tx, filter))
    }

    /// Gets all the transactions in the mempool
    pub fn get_all_txs_by_sender(
        &self,
    ) -> Result<FxHashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let mut txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>> =
            FxHashMap::with_capacity_and_hasher(128, Default::default());
        let tx_pool = &self.read()?.transaction_pool;

        for (_, tx) in tx_pool.iter() {
            txs_by_sender
                .entry(tx.sender())
                .or_insert_with(|| Vec::with_capacity(128))
                .push(tx.clone())
        }

        txs_by_sender.iter_mut().for_each(|(_, txs)| txs.sort());
        Ok(txs_by_sender)
    }

    /// Applies the filter and returns a set of suitable transactions from the mempool.
    /// These transactions will be grouped by sender and sorted by nonce
    pub fn filter_transactions_with_filter_fn(
        &self,
        filter: &dyn Fn(&Transaction) -> bool,
    ) -> Result<FxHashMap<Address, Vec<MempoolTransaction>>, StoreError> {
        let (mut txs_by_sender, _) = self.filter_transactions_with_order(filter)?;
        txs_by_sender.iter_mut().for_each(|(_, txs)| txs.sort());
        Ok(txs_by_sender)
    }

    /// Applies the filter and returns suitable transactions grouped by sender, together with
    /// their mempool insertion order.
    pub fn filter_transactions_with_order(
        &self,
        filter: &dyn Fn(&Transaction) -> bool,
    ) -> Result<
        (
            FxHashMap<Address, Vec<MempoolTransaction>>,
            FxHashMap<H256, usize>,
        ),
        StoreError,
    > {
        let mut txs_by_sender: FxHashMap<Address, Vec<MempoolTransaction>> =
            FxHashMap::with_capacity_and_hasher(128, Default::default());
        let mut insertion_order: FxHashMap<H256, usize> =
            FxHashMap::with_capacity_and_hasher(128, Default::default());
        let inner = self.read()?;

        for (order, hash) in inner.txs_order.iter().enumerate() {
            let Some(tx) = inner.transaction_pool.get(hash) else {
                continue;
            };
            if filter(tx) {
                txs_by_sender
                    .entry(tx.sender())
                    .or_insert_with(|| Vec::with_capacity(128))
                    .push(tx.clone());
                insertion_order.entry(*hash).or_insert(order);
            }
        }

        txs_by_sender.iter_mut().for_each(|(_, txs)| txs.sort());
        Ok((txs_by_sender, insertion_order))
    }

    /// Filters hashes to those not already in the mempool or in-flight, and
    /// atomically marks the returned hashes as in-flight under a single write
    /// lock so that concurrent peer handlers cannot request the same hashes.
    pub fn reserve_unknown_hashes(
        &self,
        possible_hashes: &[H256],
    ) -> Result<Vec<H256>, StoreError> {
        let mut inner = self.write()?;

        let unknown: Vec<H256> = possible_hashes
            .iter()
            .filter(|hash| {
                !inner.in_flight_txs.contains(hash) && !inner.transaction_pool.contains_key(hash)
            })
            .copied()
            .collect();

        inner.in_flight_txs.extend(unknown.iter().copied());
        Ok(unknown)
    }

    /// Removes transaction hashes from the in-flight set, typically called
    /// when the GetPooledTransactions response arrives (or the connection drops).
    pub fn clear_in_flight_txs(&self, hashes: &[H256]) -> Result<(), StoreError> {
        let mut inner = self.write()?;
        for hash in hashes {
            inner.in_flight_txs.remove(hash);
        }
        Ok(())
    }

    pub fn get_transaction_by_hash(
        &self,
        transaction_hash: H256,
    ) -> Result<Option<Transaction>, StoreError> {
        let tx = self
            .read()?
            .transaction_pool
            .get(&transaction_hash)
            .map(|e| e.transaction().clone());

        Ok(tx)
    }

    pub fn get_nonce(&self, address: &Address) -> Result<Option<u64>, MempoolError> {
        Ok(self
            .read()?
            .txs_by_sender_nonce
            .range((*address, 0)..=(*address, u64::MAX))
            .last()
            .map(|((_address, nonce), _hash)| nonce + 1))
    }

    pub fn get_mempool_size(&self) -> Result<(u64, u64), MempoolError> {
        let txs_size = {
            let pool_lock = &self.read()?.transaction_pool;
            pool_lock.len()
        };
        let blobs_size = {
            let pool_lock = &self.read()?.blobs_bundle_pool;
            pool_lock.len()
        };

        Ok((txs_size as u64, blobs_size as u64))
    }

    /// Returns all transactions currently in the pool
    pub fn content(&self) -> Result<Vec<Transaction>, MempoolError> {
        let pooled_transactions = &self.read()?.transaction_pool;
        Ok(pooled_transactions
            .values()
            .map(MempoolTransaction::transaction)
            .cloned()
            .collect())
    }

    /// Returns all blobs bundles currently in the pool
    pub fn get_blobs_bundle_pool(&self) -> Result<Vec<BlobsBundle>, MempoolError> {
        let blobs_bundle_pool = &self.read()?.blobs_bundle_pool;
        Ok(blobs_bundle_pool.values().cloned().collect())
    }

    /// Returns blobs data (blob, commitment, proof) associated with the versioned hashes
    pub fn get_blobs_data_by_versioned_hashes(
        &self,
        versioned_hashes: &[H256],
    ) -> Result<Vec<Option<BlobTuple>>, MempoolError> {
        let mempool = self.read()?;
        let blobs_bundle_pool = &mempool.blobs_bundle_pool;
        let blobs_bundle_by_versioned_hash = &mempool.blobs_bundle_by_versioned_hash;
        let mut res = vec![None; versioned_hashes.len()];
        for (idx, vh) in versioned_hashes.iter().enumerate() {
            if let Some((found_hash, inner_pos)) = blobs_bundle_by_versioned_hash
                .get(vh)
                .and_then(|h| h.iter().next())
            {
                res[idx] = blobs_bundle_pool
                    .get(found_hash)
                    .and_then(|b| b.get_blob_tuple_by_index(*inner_pos))
            }
        }
        Ok(res)
    }

    /// Returns the status of the mempool, which is the number of transactions currently in
    /// the pool. Until we add "queue" transactions.
    pub fn status(&self) -> Result<u64, MempoolError> {
        let pool_lock = &self.read()?.transaction_pool;

        Ok(pool_lock.len() as u64)
    }

    pub fn contains_sender_nonce(
        &self,
        sender: Address,
        nonce: u64,
        received_hash: H256,
    ) -> Result<Option<MempoolTransaction>, MempoolError> {
        let Some(hash) = self
            .read()?
            .txs_by_sender_nonce
            .get(&(sender, nonce))
            .cloned()
        else {
            return Ok(None);
        };
        if hash == received_hash {
            return Ok(None);
        }

        let transaction_pool = &self.read()?.transaction_pool;
        let tx = transaction_pool.get(&hash).cloned();
        Ok(tx)
    }

    pub fn contains_tx(&self, tx_hash: H256) -> Result<bool, MempoolError> {
        let contains = self.read()?.transaction_pool.contains_key(&tx_hash);
        Ok(contains)
    }

    pub fn find_tx_to_replace(
        &self,
        sender: Address,
        nonce: u64,
        tx: &Transaction,
    ) -> Result<Option<H256>, MempoolError> {
        let Some(tx_in_pool) = self.contains_sender_nonce(sender, nonce, tx.hash())? else {
            return Ok(None);
        };
        if is_replacement_underpriced(tx_in_pool.transaction(), tx) {
            return Err(MempoolError::UnderpricedReplacement);
        }

        Ok(Some(tx_in_pool.hash()))
    }
}

fn is_replacement_underpriced(existing: &Transaction, replacement: &Transaction) -> bool {
    let price_bump = replacement_price_bump(existing.tx_type());

    if replacement_max_fee_per_gas(replacement)
        < bumped_price(replacement_max_fee_per_gas(existing), price_bump)
    {
        return true;
    }

    let existing_priority_fee = replacement_priority_fee_per_gas(existing);
    let replacement_priority_fee = replacement_priority_fee_per_gas(replacement);
    if existing_priority_fee != U256::zero()
        && replacement_priority_fee != U256::zero()
        && replacement_priority_fee < bumped_price(existing_priority_fee, price_bump)
    {
        return true;
    }

    if let Some(existing_blob_fee) = existing.max_fee_per_blob_gas() {
        let replacement_blob_fee = replacement.max_fee_per_blob_gas().unwrap_or_default();
        if replacement_blob_fee < bumped_price(existing_blob_fee, price_bump) {
            return true;
        }
    }

    false
}

fn replacement_price_bump(tx_type: TxType) -> u128 {
    if tx_type == TxType::EIP4844 {
        BLOB_REPLACEMENT_PRICE_BUMP
    } else {
        DEFAULT_REPLACEMENT_PRICE_BUMP
    }
}

fn replacement_max_fee_per_gas(tx: &Transaction) -> U256 {
    tx.max_fee_per_gas()
        .map(U256::from)
        .unwrap_or_else(|| tx.gas_price())
}

fn replacement_priority_fee_per_gas(tx: &Transaction) -> U256 {
    tx.max_priority_fee()
        .map(U256::from)
        .unwrap_or_else(U256::zero)
}

fn bumped_price(price: U256, bump_percent: u128) -> U256 {
    U256::saturating_mul(price, U256::from(100 + bump_percent)) / U256::from(100)
}

#[derive(Debug, Default)]
pub struct PendingTxFilter {
    pub min_tip: Option<u64>,
    pub base_fee: Option<u64>,
    pub blob_fee: Option<u64>,
    pub only_plain_txs: bool,
    pub only_blob_txs: bool,
}

fn pending_tx_matches_filter(tx: &Transaction, filter: &PendingTxFilter) -> bool {
    // Filter by tx type
    let is_blob_tx = matches!(tx, Transaction::EIP4844Transaction(_));
    if filter.only_plain_txs && is_blob_tx || filter.only_blob_txs && !is_blob_tx {
        return false;
    }

    // Filter by tip & base_fee
    if let Some(min_tip) = filter.min_tip.map(U256::from) {
        if tx
            .effective_gas_tip(filter.base_fee)
            .is_none_or(|tip| tip < min_tip)
        {
            return false;
        }
    // This is a temporary fix to avoid invalid transactions to be included.
    // This should be removed once https://github.com/lambdaclass/ethrex/issues/680
    // is addressed.
    } else if tx.effective_gas_tip(filter.base_fee).is_none() {
        return false;
    }

    // Filter by blob gas fee
    if is_blob_tx
        && let Some(blob_fee) = filter.blob_fee
        && tx
            .max_fee_per_blob_gas()
            .is_none_or(|fee| fee < blob_fee.into())
    {
        return false;
    }
    true
}

pub fn transaction_intrinsic_gas(
    tx: &Transaction,
    header: &BlockHeader,
    config: &ChainConfig,
) -> Result<u64, MempoolError> {
    let is_contract_creation = tx.is_contract_creation();

    let mut gas = if is_contract_creation {
        TX_CREATE_GAS_COST
    } else {
        TX_GAS_COST
    };

    let data_len = tx.data().len() as u64;

    if data_len > 0 {
        let non_zero_gas_cost = if config.is_istanbul_activated(header.number) {
            TX_DATA_NON_ZERO_GAS_EIP2028
        } else {
            TX_DATA_NON_ZERO_GAS
        };

        let non_zero_count = tx.data().iter().filter(|&&x| x != 0u8).count() as u64;

        gas = gas
            .checked_add(non_zero_count * non_zero_gas_cost)
            .ok_or(MempoolError::TxGasOverflowError)?;

        let zero_count = data_len - non_zero_count;

        gas = gas
            .checked_add(zero_count * TX_DATA_ZERO_GAS_COST)
            .ok_or(MempoolError::TxGasOverflowError)?;

        if is_contract_creation && config.is_shanghai_activated(header.timestamp) {
            // Len in 32 bytes sized words
            let len_in_words = data_len.saturating_add(31) / 32;

            gas = gas
                .checked_add(len_in_words * TX_INIT_CODE_WORD_GAS_COST)
                .ok_or(MempoolError::TxGasOverflowError)?;
        }
    }

    let storage_keys_count: u64 = tx
        .access_list()
        .iter()
        .map(|(_, keys)| keys.len() as u64)
        .sum();

    gas = gas
        .checked_add(tx.access_list().len() as u64 * TX_ACCESS_LIST_ADDRESS_GAS)
        .ok_or(MempoolError::TxGasOverflowError)?;

    gas = gas
        .checked_add(storage_keys_count * TX_ACCESS_LIST_STORAGE_KEY_GAS)
        .ok_or(MempoolError::TxGasOverflowError)?;

    Ok(gas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ethrex_common::types::{EIP1559Transaction, LegacyTransaction, MempoolTransaction};

    fn legacy_tx(nonce: u64, gas_price: u64) -> Transaction {
        Transaction::LegacyTransaction(LegacyTransaction {
            nonce,
            gas_price: U256::from(gas_price),
            gas: 21_000,
            ..Default::default()
        })
    }

    fn eip1559_tx(nonce: u64, max_fee_per_gas: u64, max_priority_fee_per_gas: u64) -> Transaction {
        Transaction::EIP1559Transaction(EIP1559Transaction {
            nonce,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            gas_limit: 21_000,
            ..Default::default()
        })
    }

    fn add_existing_tx(mempool: &Mempool, sender: Address, tx: Transaction) -> H256 {
        let mempool_tx = MempoolTransaction::new(tx, sender);
        let hash = mempool_tx.transaction().hash();
        mempool.add_transaction(hash, sender, mempool_tx).unwrap();
        hash
    }

    #[test]
    fn clear_removes_pending_transactions() {
        let mempool = Mempool::new(10);
        let sender = Address::default();
        let tx = Transaction::LegacyTransaction(LegacyTransaction::default());
        let mempool_tx = MempoolTransaction::new(tx, sender);
        let hash = mempool_tx.transaction().hash();

        mempool.add_transaction(hash, sender, mempool_tx).unwrap();
        assert_eq!(mempool.get_mempool_size().unwrap(), (1, 0));

        mempool.clear().unwrap();

        assert_eq!(mempool.get_mempool_size().unwrap(), (0, 0));
        assert!(mempool.content().unwrap().is_empty());
    }

    #[test]
    fn replacement_rejects_same_price_eip1559_over_legacy() {
        let mempool = Mempool::new(10);
        let sender = Address::default();
        add_existing_tx(&mempool, sender, legacy_tx(7, 100));

        let result = mempool.find_tx_to_replace(sender, 7, &eip1559_tx(7, 100, 100));

        assert!(matches!(result, Err(MempoolError::UnderpricedReplacement)));
    }

    #[test]
    fn replacement_accepts_ten_percent_bump_eip1559_over_legacy() {
        let mempool = Mempool::new(10);
        let sender = Address::default();
        let existing_hash = add_existing_tx(&mempool, sender, legacy_tx(7, 100));

        let result = mempool.find_tx_to_replace(sender, 7, &eip1559_tx(7, 110, 110));

        assert_eq!(result.unwrap(), Some(existing_hash));
    }

    #[test]
    fn replacement_rejects_eip1559_without_priority_fee_bump() {
        let mempool = Mempool::new(10);
        let sender = Address::default();
        add_existing_tx(&mempool, sender, eip1559_tx(7, 100, 100));

        let result = mempool.find_tx_to_replace(sender, 7, &eip1559_tx(7, 110, 109));

        assert!(matches!(result, Err(MempoolError::UnderpricedReplacement)));
    }

    #[test]
    fn replacement_accepts_legacy_with_ten_percent_max_fee_bump() {
        let mempool = Mempool::new(10);
        let sender = Address::default();
        let existing_hash = add_existing_tx(&mempool, sender, eip1559_tx(7, 100, 100));

        let result = mempool.find_tx_to_replace(sender, 7, &legacy_tx(7, 110));

        assert_eq!(result.unwrap(), Some(existing_hash));
    }
}
