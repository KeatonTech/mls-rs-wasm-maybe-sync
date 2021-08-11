#![allow(clippy::nonstandard_macro_braces)]
#![allow(clippy::enum_variant_names)]

#[cfg(test)]
macro_rules! hex {
    ($input:literal) => {
        hex::decode($input).expect("invalid hex value")
    };
}

#[macro_use]
pub mod ciphersuite;
pub mod client;
pub mod credential;
pub mod extension;
pub mod group;
pub mod key_package;
pub mod protocol_version;
pub mod session;
pub mod tree_kem;
