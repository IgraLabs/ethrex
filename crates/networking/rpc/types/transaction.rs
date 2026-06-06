use crate::utils::RpcErr;
#[cfg(feature = "falcon-l5")]
use ethrex_common::types::{IGRA_FALCON_L5_TX_TYPE, IgraFalconL5Transaction};
use ethrex_common::{
    Address, H256, serde_utils,
    types::{
        BlockHash, BlockNumber, EIP1559Transaction, EIP2930Transaction, EIP7702Transaction,
        FeeTokenTransaction, LegacyTransaction, PrivilegedL2Transaction, Transaction,
        WrappedEIP4844Transaction,
    },
};
use ethrex_crypto::NativeCrypto;
use ethrex_rlp::{decode::RLPDecode, error::RLPDecodeError};
use serde::{Deserialize, Serialize};

#[allow(unused)]
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcTransaction {
    #[serde(flatten)]
    pub tx: Transaction,
    #[serde(with = "serde_utils::u64::hex_str_opt")]
    block_number: Option<BlockNumber>,
    block_hash: Option<BlockHash>,
    from: Address,
    pub hash: H256,
    #[serde(with = "serde_utils::u64::hex_str_opt")]
    transaction_index: Option<u64>,
}

impl RpcTransaction {
    pub fn build(
        tx: Transaction,
        block_number: Option<BlockNumber>,
        block_hash: Option<BlockHash>,
        transaction_index: Option<usize>,
    ) -> Result<Self, RpcErr> {
        let from = tx.sender(&NativeCrypto)?;
        let hash = tx.hash();
        let transaction_index = transaction_index.map(|n| n as u64);
        Ok(RpcTransaction {
            tx,
            block_number,
            block_hash,
            from,
            hash,
            transaction_index,
        })
    }
}

#[derive(Debug)]
pub enum SendRawTransactionRequest {
    Legacy(LegacyTransaction),
    EIP2930(EIP2930Transaction),
    EIP1559(EIP1559Transaction),
    EIP4844(WrappedEIP4844Transaction),
    EIP7702(EIP7702Transaction),
    PrivilegedL2(PrivilegedL2Transaction),
    FeeToken(FeeTokenTransaction),
    #[cfg(feature = "falcon-l5")]
    IgraFalconL5 {
        raw: IgraFalconL5Transaction,
        execution: PrivilegedL2Transaction,
    },
}

impl SendRawTransactionRequest {
    pub fn to_transaction(&self) -> Transaction {
        match self {
            SendRawTransactionRequest::Legacy(t) => Transaction::LegacyTransaction(t.clone()),
            SendRawTransactionRequest::EIP1559(t) => Transaction::EIP1559Transaction(t.clone()),
            SendRawTransactionRequest::EIP2930(t) => Transaction::EIP2930Transaction(t.clone()),
            SendRawTransactionRequest::EIP4844(t) => Transaction::EIP4844Transaction(t.tx.clone()),
            SendRawTransactionRequest::EIP7702(t) => Transaction::EIP7702Transaction(t.clone()),
            SendRawTransactionRequest::PrivilegedL2(t) => {
                Transaction::PrivilegedL2Transaction(t.clone())
            }
            SendRawTransactionRequest::FeeToken(t) => Transaction::FeeTokenTransaction(t.clone()),
            #[cfg(feature = "falcon-l5")]
            SendRawTransactionRequest::IgraFalconL5 { execution, .. } => {
                Transaction::PrivilegedL2Transaction(execution.clone())
            }
        }
    }

    pub fn decode_canonical(bytes: &[u8]) -> Result<Self, RLPDecodeError> {
        // Look at the first byte to check if it corresponds to a TransactionType
        match bytes.first() {
            // First byte is a valid TransactionType https://eips.ethereum.org/EIPS/eip-2718#transactiontype-only-goes-up-to-0x7f
            Some(tx_type) if *tx_type <= 0x7f => {
                // Decode tx based on type
                let tx_bytes = &bytes[1..];

                match *tx_type {
                    // Legacy
                    0x0 => {
                        LegacyTransaction::decode(tx_bytes).map(SendRawTransactionRequest::Legacy)
                    }
                    // EIP2930
                    0x1 => {
                        EIP2930Transaction::decode(tx_bytes).map(SendRawTransactionRequest::EIP2930)
                    }
                    // EIP1559
                    0x2 => {
                        EIP1559Transaction::decode(tx_bytes).map(SendRawTransactionRequest::EIP1559)
                    }
                    // EIP4844
                    0x3 => WrappedEIP4844Transaction::decode(tx_bytes)
                        .map(SendRawTransactionRequest::EIP4844),
                    // EIP7702
                    0x4 => {
                        EIP7702Transaction::decode(tx_bytes).map(SendRawTransactionRequest::EIP7702)
                    }
                    // Igra Falcon-L5 q transaction
                    #[cfg(feature = "falcon-l5")]
                    IGRA_FALCON_L5_TX_TYPE => {
                        let raw = IgraFalconL5Transaction::decode(tx_bytes)?;
                        let execution = raw.to_privileged_transaction().map_err(|error| {
                            RLPDecodeError::Custom(format!("Invalid Falcon-L5 q tx: {error}"))
                        })?;
                        Ok(SendRawTransactionRequest::IgraFalconL5 { raw, execution })
                    }
                    // FeeTokenTransaction
                    0x7d => FeeTokenTransaction::decode(tx_bytes)
                        .map(SendRawTransactionRequest::FeeToken),
                    // PrivilegedL2Transaction
                    0x7e => PrivilegedL2Transaction::decode(tx_bytes)
                        .map(SendRawTransactionRequest::PrivilegedL2),
                    ty => Err(RLPDecodeError::Custom(format!(
                        "Invalid transaction type: {ty}"
                    ))),
                }
            }
            // LegacyTransaction
            _ => LegacyTransaction::decode(bytes).map(SendRawTransactionRequest::Legacy),
        }
    }
}

#[cfg(all(test, feature = "falcon-l5"))]
mod tests {
    use super::*;
    use bytes::Bytes;
    use ethrex_common::{
        U256,
        types::{IGRA_FALCON_L5_TX_TYPE, TxKind},
    };
    use ethrex_crypto::falcon_l5::{
        falcon_l5_auth_bytes, falcon_l5_pubkey_to_address, generate_falcon_l5_keypair,
    };

    fn signed_q_tx() -> (IgraFalconL5Transaction, Address) {
        let (sk, pk) = generate_falcon_l5_keypair().expect("Falcon key generation succeeds");
        let mut tx = IgraFalconL5Transaction {
            chain_id: 2026,
            nonce: 11,
            max_priority_fee_per_gas: 2,
            max_fee_per_gas: 9,
            gas_limit: 75_000,
            to: TxKind::Call(Address::from_low_u64_be(0x77)),
            value: U256::from(1u64),
            data: Bytes::from_static(b"q-rpc"),
            access_list: vec![],
            falcon_auth: Bytes::new(),
            ..Default::default()
        };

        let signing_hash = tx.signing_hash().to_fixed_bytes();
        let sig = sk.sign_ct(&signing_hash).expect("Falcon signing succeeds");
        tx.falcon_auth = Bytes::copy_from_slice(&falcon_l5_auth_bytes(&pk, &sig));

        (tx, falcon_l5_pubkey_to_address(&pk))
    }

    #[test]
    fn decode_canonical_accepts_falcon_l5_q_tx() {
        let (tx, sender) = signed_q_tx();
        let encoded = tx.encode_canonical_to_vec();
        assert_eq!(encoded[0], IGRA_FALCON_L5_TX_TYPE);

        let decoded =
            SendRawTransactionRequest::decode_canonical(&encoded).expect("q raw tx decodes");
        let SendRawTransactionRequest::IgraFalconL5 { raw, execution } = decoded else {
            panic!("expected IgraFalconL5 request");
        };

        assert_eq!(raw.sender().expect("q sender recovers"), sender);
        assert_eq!(execution.from, sender);
        assert_eq!(execution.nonce, tx.nonce);
        assert_eq!(execution.data, tx.data);
    }

    #[test]
    fn decode_canonical_rejects_bad_falcon_l5_q_tx() {
        let (mut tx, _sender) = signed_q_tx();
        let mut auth = tx.falcon_auth.to_vec();
        auth[0] ^= 0x01;
        tx.falcon_auth = Bytes::from(auth);

        let encoded = tx.encode_canonical_to_vec();
        assert!(SendRawTransactionRequest::decode_canonical(&encoded).is_err());
    }
}
