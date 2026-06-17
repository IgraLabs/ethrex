//! IGRA shared multi-zone execution — zone model (M0).
//!
//! Per the M0 proposal (`igra-multizone-m0.md`). M0 is **additive**: it does not change the
//! existing canonical or q-zone paths. It introduces a new top-level *multi-zone* logic zone
//! that runs several internal **sub-zones** under one block finalizer.
//!
//! Two namespaces, kept explicitly distinct:
//! - [`TopLevelZoneId`] — the outer Igra/Kaspa carrier-envelope destination.
//! - [`SubZoneId`] — an internal execution namespace inside the multi-zone environment.
//!
//! M0 keeps the internal sub-zone list static: `c` (canonical-style EVM, ECDSA) + `q` (q-EVM,
//! Falcon-L5). Dynamic sub-zone creation is out of scope for M0.

use ethereum_types::{Address, H256};
use ethrex_rlp::encode::RLPEncode;
use serde::{Deserialize, Serialize};

use crate::utils::keccak;

/// Outer Igra/Kaspa carrier-envelope destination (top-level logic zone).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct TopLevelZoneId(pub u16);

/// Internal execution namespace inside the multi-zone environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SubZoneId(pub u16);

/// Internal sub-zone: canonical-style EVM (Ethereum ECDSA auth).
pub const IGRA_SUBZONE_C: SubZoneId = SubZoneId(0x0001);
/// Internal sub-zone: q-EVM (Falcon-L5 auth).
pub const IGRA_SUBZONE_Q: SubZoneId = SubZoneId(0x0002);

/// The execution semantics a sub-zone runs. M0 supports EVM-like kinds only; new VM kinds are
/// a consensus change (see M0 §7.3) and are intentionally not modeled yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VmKind {
    /// Canonical Ethereum EVM semantics.
    Evm,
    /// q-EVM: EVM semantics with q-specific auth + precompile policy (Falcon-L5).
    QEvm,
}

/// The authentication policy a sub-zone enforces on its transactions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthPolicy {
    /// Ethereum ECDSA signatures.
    Ecdsa,
    /// Falcon-L5 post-quantum signatures (q-zone).
    FalconL5,
    /// KYC gate: in addition to the zone's base auth, every tx's sender must be allow-listed in
    /// the on-chain `KycRegistry` at this address (read against the block's own pre-tx state, so
    /// the verdict is deterministic on re-derivation). See the KYC enforcement module.
    Kyc { registry: Address },
}

/// Static configuration of one internal sub-zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubZoneConfig {
    pub subzone_id: SubZoneId,
    pub vm_kind: VmKind,
    pub auth_policy: AuthPolicy,
}

/// The committed state root of one internal sub-zone at a given block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubZoneRoot {
    pub subzone_id: SubZoneId,
    pub state_root: H256,
}

/// Deterministic commitment over the multi-zone state: a hash of the sub-zone roots in a
/// canonical (ascending `subzone_id`) order. This value goes into the block header's
/// `subzone_roots_root` and changes whenever any included sub-zone root changes.
///
/// Determinism is essential: every node re-deriving the chain must compute the identical
/// commitment, so the input is sorted by `subzone_id` and RLP-encoded before hashing.
pub fn subzone_roots_root(roots: &[SubZoneRoot]) -> H256 {
    let mut ordered: Vec<SubZoneRoot> = roots.to_vec();
    ordered.sort_by_key(|r| r.subzone_id.0);

    // Encode as a flat RLP list of [subzone_id (u16), state_root (H256)] pairs, then keccak.
    let mut buf = Vec::with_capacity(ordered.len() * 34);
    for r in &ordered {
        r.subzone_id.0.encode(&mut buf);
        r.state_root.encode(&mut buf);
    }
    keccak(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(id: u16, byte: u8) -> SubZoneRoot {
        SubZoneRoot {
            subzone_id: SubZoneId(id),
            state_root: H256::repeat_byte(byte),
        }
    }

    #[test]
    fn roots_commitment_is_deterministic_and_order_independent() {
        let a = vec![root(1, 0xaa), root(2, 0xbb)];
        let b = vec![root(2, 0xbb), root(1, 0xaa)]; // same set, different input order
        assert_eq!(subzone_roots_root(&a), subzone_roots_root(&b));
    }

    #[test]
    fn roots_commitment_changes_when_a_root_changes() {
        let a = vec![root(1, 0xaa), root(2, 0xbb)];
        let b = vec![root(1, 0xaa), root(2, 0xcc)]; // q root changed
        assert_ne!(subzone_roots_root(&a), subzone_roots_root(&b));
    }

    #[test]
    fn roots_commitment_distinguishes_subzone_assignment() {
        // Same two root hashes, swapped between sub-zones, must commit differently.
        let a = vec![root(1, 0xaa), root(2, 0xbb)];
        let b = vec![root(1, 0xbb), root(2, 0xaa)];
        assert_ne!(subzone_roots_root(&a), subzone_roots_root(&b));
    }

    #[test]
    fn static_subzone_ids() {
        assert_eq!(IGRA_SUBZONE_C, SubZoneId(0x0001));
        assert_eq!(IGRA_SUBZONE_Q, SubZoneId(0x0002));
    }
}
