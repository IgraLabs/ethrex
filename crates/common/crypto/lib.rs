#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

pub mod blake2f;
#[cfg(feature = "falcon-l5")]
pub mod falcon_l5;
pub mod keccak;
pub mod kzg;
pub mod native;
pub mod provider;
pub use native::NativeCrypto;
pub use provider::{Crypto, CryptoError};
