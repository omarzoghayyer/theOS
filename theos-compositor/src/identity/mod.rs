// identity/mod.rs — theOS Cryptographic Identity Engine
// This replaces the phone number entirely.
// Your public key IS your address. No central registry.
// You can only be reached by people you have explicitly invited.

pub mod keypair;
pub mod contact;
pub mod exchange;

pub use keypair::{IdentityKey, KeyPair};
pub use contact::{Contact, ContactBook};
pub use exchange::ContactExchange;
