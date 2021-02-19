#![cfg_attr(not(feature = "std"), no_std)]
#![allow(clippy::unnecessary_cast)]

#[cfg(feature = "std")]
use serde::{Deserialize, Serialize};

use codec::{Decode, Encode};
use sp_runtime::RuntimeDebug;
use sp_std::convert::{Into, TryFrom, TryInto};

#[derive(Encode, Decode, Eq, PartialEq, Copy, Clone, RuntimeDebug, PartialOrd, Ord)]
#[cfg_attr(feature = "std", derive(Serialize, Deserialize))]
pub enum TokenSymbol {
    DOT = 0,
    BDT = 1,
    BUSD = 2,
    BBTC = 3,
    BETH = 4,
    BDOT = 5,
}

impl TryFrom<u8> for TokenSymbol {
    type Error = ();

    fn try_from(v: u8) -> Result<Self, Self::Error> {
        match v {
            0 => Ok(TokenSymbol::DOT),
            1 => Ok(TokenSymbol::BDT),
            2 => Ok(TokenSymbol::BUSD),
            3 => Ok(TokenSymbol::BBTC),
            4 => Ok(TokenSymbol::BETH),
            5 => Ok(TokenSymbol::BDOT),
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