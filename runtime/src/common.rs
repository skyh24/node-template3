#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::unnecessary_cast)]

#[cfg(feature = "std")]
use serde::{Deserialize, Serialize};

use codec::{Decode, Encode};
use sp_std::convert::{Into, TryFrom, TryInto};
use sp_runtime::{
    generic,
    traits::{
        BlakeTwo256, Verify, IdentifyAccount
    },
    MultiSignature, RuntimeDebug,
};
use crate::UncheckedExtrinsic;

pub type BlockNumber = u32;
/// Alias to 512-bit hash when used in the context of a transaction signature on the chain.
pub type Signature = MultiSignature;

/// Alias to the public key used for this chain, actually a `MultiSigner`. Like
/// the signature, this also isn't a fixed size when encoded, as different
/// cryptos have different size public keys.
pub type AccountPublic = <Signature as Verify>::Signer;

/// Some way of identifying an account on the chain. We intentionally make it equivalent
/// to the public key of our transaction signing scheme.
pub type AccountId = <<Signature as Verify>::Signer as IdentifyAccount>::AccountId;
pub type AccountIndex = u32;
pub type Amount = i128;
pub type Balance = u128;
pub type Share = u128;
pub type Moment = u64;
pub type Nonce = u32;
pub type Index = u32;
pub type EraIndex = u32;
pub type AuctionId = u32;
pub type Hash = sp_core::H256;
pub type DigestItem = generic::DigestItem<Hash>;

pub type Address = sp_runtime::MultiAddress<AccountId, ()>;
pub type Header = generic::Header<BlockNumber, BlakeTwo256>;
pub type Block = generic::Block<Header, UncheckedExtrinsic>;
pub type BlockId = generic::BlockId<Block>;
pub type SignedBlock = generic::SignedBlock<Block>;

/// Opaque types. These are used by the CLI to instantiate machinery that don't need to know
/// the specifics of the runtime. They can then be made to be agnostic over specific formats
/// of data like extrinsics, allowing for them to continue syncing the network through upgrades
/// to even the core data structures.
pub mod opaque {
    use super::*;
    pub use sp_runtime::OpaqueExtrinsic as UncheckedExtrinsic;

    /// Opaque block header type.
    pub type Header = generic::Header<BlockNumber, BlakeTwo256>;
    /// Opaque block type.
    pub type Block = generic::Block<Header, UncheckedExtrinsic>;
    /// Opaque block identifier type.
    pub type BlockId = generic::BlockId<Block>;
}

#[derive(Encode, Decode, Eq, PartialEq, Copy, Clone, RuntimeDebug, PartialOrd, Ord)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub enum TokenSymbol {
    DOT = 0,
    BDT = 1,
    BUSD = 2,
    BBTC = 3,
    BDOT = 4,
}

impl TryFrom<u8> for TokenSymbol {
    type Error = ();

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(TokenSymbol::DOT),
            1 => Ok(TokenSymbol::BDT),
            2 => Ok(TokenSymbol::BUSD),
            3 => Ok(TokenSymbol::BBTC),
            4 => Ok(TokenSymbol::BDOT),
            _ => Err(()),
        }
    }
}


#[derive(Encode, Decode, Eq, PartialEq, Copy, Clone, RuntimeDebug, PartialOrd, Ord)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub enum CurrencyId {
    Token(TokenSymbol),
    DEXShare(TokenSymbol, TokenSymbol),
}

impl TryFrom<[u8; 32]> for CurrencyId {
    type Error = ();

    fn try_from(v: [u8; 32]) -> Result<Self, Self::Error> {
        if !v.starts_with(&[0u8; 29][..]) {
            return Err(());
        }

        // token
        if v[29] == 0 && v[31] == 0 {
            return v[30].try_into().map(CurrencyId::Token);
        }

        // DEX share
        if v[29] == 1 {
            let left = v[30].try_into()?;
            let right = v[31].try_into()?;
            return Ok(CurrencyId::DEXShare(left, right));
        }

        Err(())
    }
}

impl Into<[u8; 32]> for CurrencyId {
    fn into(self) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        match self {
            CurrencyId::Token(token) => {
                bytes[30] = token as u8;
            }
            CurrencyId::DEXShare(left, right) => {
                bytes[29] = 1;
                bytes[30] = left as u8;
                bytes[31] = right as u8;
            }
        }
        bytes
    }
}
