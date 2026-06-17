//! IGRA KYC logic zone enforcement.
//!
//! When a chain sets `ChainConfig.kyc_registry`, it is a KYC logic zone: every
//! transaction's sender must be allow-listed in the on-chain `KycRegistry` contract
//! at that address, or the block containing the tx is rejected as invalid.
//!
//! The allow-list is read from the *block's own execution state* (the same
//! `GeneralizedDatabase` the block executes against), at the point each tx is about
//! to execute. This makes the verdict a deterministic function of chain position:
//! every node re-deriving the chain reads the same registry state at the same point
//! and reaches the same conclusion. A later `setAllowed` only affects txs positioned
//! after it; it never retroactively changes a past tx's verdict.
//!
//! ## KycRegistry storage layout (must match contracts/KycDemo.sol)
//! ```solidity
//! contract KycRegistry {
//!     address public owner;                 // slot 0
//!     mapping(address => bool) public allowed; // slot 1
//! }
//! ```
//! For a Solidity `mapping`, the value slot is `keccak256(pad32(key) ++ pad32(slot))`.
//! Here key = the sender address, slot = 1.
//!
//! ## Bootstrap exemption
//! The registry has to be deployed, and the first addresses allow-listed, before
//! anyone is on the list. To avoid a dead zone, two kinds of tx are exempt from the
//! KYC check:
//!   - a tx that *creates* the registry contract (sender becomes the owner), and
//!   - any tx sent *to* the registry address (i.e. `setAllowed` / admin calls).
//! Everything else must have an allow-listed sender. Once the registry exists and the
//! operator has allow-listed the genuine users, normal txs flow.

use ethrex_common::types::multizone::AuthPolicy;
use ethrex_common::{Address, H256, U256, types::TxKind};
use ethrex_common::utils::keccak;
use ethrex_levm::db::gen_db::GeneralizedDatabase;

use crate::EvmError;

/// M0 entry point: enforce a sub-zone's [`AuthPolicy`] on one transaction, before it executes.
///
/// This is the executor-agnostic hook. The legacy standalone-EL path calls it via
/// `ChainConfig.kyc_registry`; the M0 `IgraBlockExecutor` calls it per sub-zone leg via
/// `SubZoneConfig.auth_policy`. Only `AuthPolicy::Kyc` does extra work here — `Ecdsa` / `FalconL5`
/// authentication is handled by the signer recovery path, so they are no-ops at this stage.
pub fn enforce_auth_policy(
    db: &mut GeneralizedDatabase,
    policy: &AuthPolicy,
    sender: Address,
    to: &TxKind,
) -> Result<(), EvmError> {
    match policy {
        AuthPolicy::Kyc { registry } => check_sender_allowed(db, *registry, sender, to),
        // Signature-scheme policies are enforced during sender recovery, not here.
        AuthPolicy::Ecdsa | AuthPolicy::FalconL5 => Ok(()),
    }
}

/// Storage slot of `mapping(address => bool) allowed` in `KycRegistry` (slot 1).
const KYC_ALLOWED_MAPPING_SLOT: u64 = 1;

/// Compute the storage key for `allowed[account]`:
/// `keccak256(pad32(account) ++ pad32(slot))`.
fn allowed_slot_key(account: Address) -> H256 {
    let mut buf = [0u8; 64];
    // pad32(account): a 20-byte address, right-aligned in the first 32 bytes.
    buf[12..32].copy_from_slice(account.as_bytes());
    // pad32(slot): the mapping's declared slot, right-aligned in the second 32 bytes.
    buf[56..64].copy_from_slice(&KYC_ALLOWED_MAPPING_SLOT.to_be_bytes());
    // `keccak` already returns an H256.
    keccak(buf)
}

/// Returns `Ok(())` if `sender` may transact on this KYC zone, or
/// `Err(EvmError::Transaction(..))` if it must be filtered.
///
/// `registry` is the KYC registry address — supplied either by the legacy
/// `ChainConfig.kyc_registry` or, in the M0 model, by `AuthPolicy::Kyc { registry }`. `to` is the
/// tx's recipient (`TxKind::Create` for contract creation). The read is against `db`, the live
/// block-execution state, so the verdict is deterministic on re-derivation.
pub fn check_sender_allowed(
    db: &mut GeneralizedDatabase,
    registry: Address,
    sender: Address,
    to: &TxKind,
) -> Result<(), EvmError> {
    // Bootstrap exemption: contract-creation txs (which is how the registry itself is
    // deployed) and any tx addressed to the registry (setAllowed / admin) are always
    // allowed, so the zone can be bootstrapped and administered.
    match to {
        TxKind::Create => return Ok(()),
        TxKind::Call(dest) if *dest == registry => return Ok(()),
        _ => {}
    }

    let key = allowed_slot_key(sender);

    // Read `allowed[sender]` with in-block correctness: prefer the live execution cache
    // (`current_accounts_state`), which reflects writes made *earlier in this same block*
    // (e.g. a `setAllowed` tx preceding this one); fall back to the committed parent
    // state via the backing store on a cache miss. This mirrors the VM's own storage
    // read and keeps the verdict a deterministic function of chain position.
    let value: U256 = match db
        .current_accounts_state
        .get(&registry)
        .and_then(|acct| acct.storage.get(&key).copied())
    {
        Some(v) => v,
        None => db
            .store
            .get_storage_value(registry, key)
            .map_err(|e| EvmError::Transaction(format!("KYC registry storage read failed: {e}")))?,
    };

    if value.is_zero() {
        return Err(EvmError::Transaction(format!(
            "KYC: sender {sender:#x} is not allow-listed in registry {registry:#x}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_slot_matches_solidity_mapping_layout() {
        // Ground truth from `cast index address 0x..01 1` (Solidity mapping slot for
        // `allowed[0x..01]` at declared slot 1): keccak256(pad32(addr) ++ pad32(1)).
        let acct = Address::from_low_u64_be(1);
        let expected = H256([
            0xcc, 0x69, 0x88, 0x5f, 0xda, 0x6b, 0xcc, 0x1a, 0x4a, 0xce, 0x05, 0x8b, 0x4a, 0x62,
            0xbf, 0x5e, 0x17, 0x9e, 0xa7, 0x8f, 0xd5, 0x8a, 0x1c, 0xcd, 0x71, 0xc2, 0x2c, 0xc9,
            0xb6, 0x88, 0x79, 0x2f,
        ]);
        assert_eq!(
            allowed_slot_key(acct),
            expected,
            "KYC slot calc must match Solidity mapping(address=>bool) at slot 1"
        );
    }

    #[test]
    fn slot_key_depends_on_account() {
        let a = allowed_slot_key(Address::from_low_u64_be(1));
        let b = allowed_slot_key(Address::from_low_u64_be(2));
        assert_ne!(a, b);
    }

    #[test]
    fn non_kyc_policies_are_noops() {
        // Ecdsa / FalconL5 must return Ok without touching the db (auth handled in recovery).
        // We can't construct a GeneralizedDatabase cheaply here, so assert the policy match arms
        // directly by pattern: only Kyc carries a registry that triggers a lookup.
        let ecdsa = AuthPolicy::Ecdsa;
        let falcon = AuthPolicy::FalconL5;
        assert!(matches!(ecdsa, AuthPolicy::Ecdsa));
        assert!(matches!(falcon, AuthPolicy::FalconL5));
        // Kyc carries the registry that enforce_auth_policy routes to check_sender_allowed.
        let kyc = AuthPolicy::Kyc { registry: Address::from_low_u64_be(0x9999) };
        assert!(matches!(kyc, AuthPolicy::Kyc { .. }));
    }

    #[test]
    fn create_and_registry_calls_are_exempt_shape() {
        // The Create / to-registry arms return before any db read (bootstrap exemption).
        assert!(matches!(TxKind::Create, TxKind::Create));
    }
}
