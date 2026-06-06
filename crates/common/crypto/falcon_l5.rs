use ethereum_types::Address;
use falcon_det::{
    det1024::{
        CtSignature, FALCON_DET1024_PRIVKEY_SIZE, FALCON_DET1024_PUBKEY_SIZE,
        FALCON_DET1024_SIG_CT_SIZE, SigningKey, VerifyingKey, generate_keypair,
    },
    shake256::Shake256Context,
};
use thiserror::Error;

pub const FALCON_L5_PUBLIC_KEY_LEN: usize = FALCON_DET1024_PUBKEY_SIZE;
pub const FALCON_L5_PRIVATE_KEY_LEN: usize = FALCON_DET1024_PRIVKEY_SIZE;
pub const FALCON_L5_CT_SIGNATURE_LEN: usize = FALCON_DET1024_SIG_CT_SIZE;
pub const FALCON_L5_AUTH_LEN: usize = FALCON_L5_PUBLIC_KEY_LEN + FALCON_L5_CT_SIGNATURE_LEN;
const ADDRESS_DOMAIN: &[u8] = b"IGRA_FALCON_L5_ADDR_V1";

#[derive(Debug, Error)]
pub enum FalconL5Error {
    #[error("invalid public key length: {0}")]
    InvalidPublicKeyLen(usize),
    #[error("invalid private key length: {0}")]
    InvalidPrivateKeyLen(usize),
    #[error("invalid signature length: {0}")]
    InvalidSignatureLen(usize),
    #[error("invalid auth length: {0}")]
    InvalidAuthLen(usize),
    #[error("signature verification failed")]
    InvalidSignature,
    #[error("signature creation failed")]
    SignFailed,
    #[error("key generation failed")]
    KeygenFailed,
}

#[derive(Clone, Debug)]
pub struct FalconL5PrivateKey {
    inner: SigningKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FalconL5PublicKey {
    inner: VerifyingKey,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FalconL5CtSignature {
    inner: CtSignature,
}

impl FalconL5PrivateKey {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FalconL5Error> {
        if bytes.len() != FALCON_L5_PRIVATE_KEY_LEN {
            return Err(FalconL5Error::InvalidPrivateKeyLen(bytes.len()));
        }

        let inner = SigningKey::from_slice(bytes)
            .map_err(|_| FalconL5Error::InvalidPrivateKeyLen(bytes.len()))?;
        Ok(Self { inner })
    }

    pub fn public_key(&self) -> FalconL5PublicKey {
        FalconL5PublicKey {
            inner: self.inner.verifying_key(),
        }
    }

    pub fn sign_ct(&self, msg: &[u8]) -> Result<FalconL5CtSignature, FalconL5Error> {
        let compressed = self
            .inner
            .sign_compressed(msg)
            .map_err(|_| FalconL5Error::SignFailed)?;
        let inner = CtSignature::try_from(compressed).map_err(|_| FalconL5Error::SignFailed)?;
        Ok(FalconL5CtSignature { inner })
    }
}

impl FalconL5PublicKey {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FalconL5Error> {
        if bytes.len() != FALCON_L5_PUBLIC_KEY_LEN {
            return Err(FalconL5Error::InvalidPublicKeyLen(bytes.len()));
        }

        let inner = VerifyingKey::from_slice(bytes)
            .map_err(|_| FalconL5Error::InvalidPublicKeyLen(bytes.len()))?;
        Ok(Self { inner })
    }

    pub fn to_bytes(self) -> [u8; FALCON_L5_PUBLIC_KEY_LEN] {
        let mut out = [0u8; FALCON_L5_PUBLIC_KEY_LEN];
        out.copy_from_slice(self.inner.as_ref());
        out
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_ref()
    }

    pub fn verify_ct(&self, msg: &[u8], sig: &FalconL5CtSignature) -> bool {
        self.inner.verify_ct(msg, &sig.inner).is_ok()
    }
}

impl FalconL5CtSignature {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, FalconL5Error> {
        if bytes.len() != FALCON_L5_CT_SIGNATURE_LEN {
            return Err(FalconL5Error::InvalidSignatureLen(bytes.len()));
        }

        let inner = CtSignature::from_slice(bytes)
            .map_err(|_| FalconL5Error::InvalidSignatureLen(bytes.len()))?;
        Ok(Self { inner })
    }

    pub fn to_bytes(self) -> [u8; FALCON_L5_CT_SIGNATURE_LEN] {
        let mut out = [0u8; FALCON_L5_CT_SIGNATURE_LEN];
        out.copy_from_slice(self.inner.as_ref());
        out
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_ref()
    }
}

pub fn generate_falcon_l5_keypair() -> Result<(FalconL5PrivateKey, FalconL5PublicKey), FalconL5Error>
{
    let mut rng =
        Shake256Context::new_prng_from_system().map_err(|_| FalconL5Error::KeygenFailed)?;
    let (sk, pk) = generate_keypair(&mut rng).map_err(|_| FalconL5Error::KeygenFailed)?;
    Ok((
        FalconL5PrivateKey { inner: sk },
        FalconL5PublicKey { inner: pk },
    ))
}

pub fn falcon_l5_pubkey_to_address(pk: &FalconL5PublicKey) -> Address {
    let mut input = Vec::with_capacity(ADDRESS_DOMAIN.len() + FALCON_L5_PUBLIC_KEY_LEN);
    input.extend_from_slice(ADDRESS_DOMAIN);
    input.extend_from_slice(pk.as_bytes());

    let hash = crate::keccak::keccak_hash(input);
    Address::from_slice(&hash[12..])
}

pub fn falcon_l5_verify(msg: &[u8], pk: &FalconL5PublicKey, sig: &FalconL5CtSignature) -> bool {
    pk.verify_ct(msg, sig)
}

pub fn falcon_l5_auth_bytes(
    pk: &FalconL5PublicKey,
    sig: &FalconL5CtSignature,
) -> [u8; FALCON_L5_AUTH_LEN] {
    let mut out = [0u8; FALCON_L5_AUTH_LEN];
    out[..FALCON_L5_PUBLIC_KEY_LEN].copy_from_slice(pk.as_bytes());
    out[FALCON_L5_PUBLIC_KEY_LEN..].copy_from_slice(sig.as_bytes());
    out
}

pub fn falcon_l5_recover_address(msg: &[u8], auth: &[u8]) -> Result<Address, FalconL5Error> {
    if auth.len() != FALCON_L5_AUTH_LEN {
        return Err(FalconL5Error::InvalidAuthLen(auth.len()));
    }

    let pk = FalconL5PublicKey::from_bytes(&auth[..FALCON_L5_PUBLIC_KEY_LEN])?;
    let sig = FalconL5CtSignature::from_bytes(&auth[FALCON_L5_PUBLIC_KEY_LEN..])?;
    if !falcon_l5_verify(msg, &pk, &sig) {
        return Err(FalconL5Error::InvalidSignature);
    }

    Ok(falcon_l5_pubkey_to_address(&pk))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deterministic_keypair() -> (FalconL5PrivateKey, FalconL5PublicKey) {
        let mut rng = Shake256Context::new_prng_from_seed(b"igra-q-logic-zone-falcon-l5-test-seed");
        let (sk, pk) = generate_keypair(&mut rng).expect("deterministic Falcon keygen succeeds");
        (
            FalconL5PrivateKey { inner: sk },
            FalconL5PublicKey { inner: pk },
        )
    }

    #[test]
    fn sizes_match_falcon_l5_auth_format() {
        assert_eq!(FALCON_L5_PUBLIC_KEY_LEN, 1793);
        assert_eq!(FALCON_L5_CT_SIGNATURE_LEN, 1538);
        assert_eq!(FALCON_L5_AUTH_LEN, 3331);
    }

    #[test]
    fn sign_verify_and_recover_address() {
        let (sk, pk) = deterministic_keypair();
        let msg = [0x42u8; 32];
        let sig = sk.sign_ct(&msg).expect("Falcon signing succeeds");
        let auth = falcon_l5_auth_bytes(&pk, &sig);

        assert!(falcon_l5_verify(&msg, &pk, &sig));
        let recovered = falcon_l5_recover_address(&msg, &auth).expect("auth recovers address");
        assert_eq!(recovered, falcon_l5_pubkey_to_address(&pk));
    }

    #[test]
    fn bad_signature_fails() {
        let (sk, pk) = deterministic_keypair();
        let msg = [0x33u8; 32];
        let mut sig = sk.sign_ct(&msg).expect("Falcon signing succeeds");
        let mut sig_bytes = sig.to_bytes();
        sig_bytes[17] ^= 0x01;
        sig = FalconL5CtSignature::from_bytes(&sig_bytes).expect("mutated sig has valid length");

        let auth = falcon_l5_auth_bytes(&pk, &sig);
        assert!(falcon_l5_recover_address(&msg, &auth).is_err());
    }

    #[test]
    fn address_derivation_is_domain_separated() {
        let (_sk, pk) = deterministic_keypair();
        let mut input = Vec::with_capacity(ADDRESS_DOMAIN.len() + FALCON_L5_PUBLIC_KEY_LEN);
        input.extend_from_slice(ADDRESS_DOMAIN);
        input.extend_from_slice(pk.as_bytes());
        let hash = crate::keccak::keccak_hash(input);
        let expected = Address::from_slice(&hash[12..]);

        assert_eq!(falcon_l5_pubkey_to_address(&pk), expected);
    }
}
