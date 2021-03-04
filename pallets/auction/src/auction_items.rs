#![cfg_attr(not(feature = "std"), no_std)]

use codec::{ Decode, Encode };
use sp_runtime::{
    traits::Zero,
    FixedPointNumber, RuntimeDebug,
};
use primitives::{Balance, Rate, CurrencyId};

#[cfg_attr(feature = "std", derive(PartialEq, Eq))]
#[derive(Encode, Decode, Clone, RuntimeDebug)]
pub struct CollateralAuctionItem<AccountId, BlockNumber> {
    /// Refund recipient for may receive refund
    pub refund_recipient: AccountId,
    pub currency_id: CurrencyId,
    #[codec(compact)]
    pub initial_amount: Balance,
    #[codec(compact)]
    pub amount: Balance,
    #[codec(compact)]
    pub target: Balance,
    /// Auction start time
    pub start_time: BlockNumber,
}

impl<AccountId, BlockNumber> CollateralAuctionItem<AccountId, BlockNumber> {
    // target = 0
    pub fn always_forward(&self) -> bool {
        self.target.is_zero()
    }

    pub fn in_reverse_stage(&self, bid_price: Balance) -> bool {
        !self.always_forward() && bid_price >= self.target
    }

    /// bid和target的最小值
    pub fn payment_amount(&self, bid_price: Balance) -> Balance {
        if self.always_forward() {
            bid_price
        } else {
            sp_std::cmp::min(self.target, bid_price)
        }
    }

    /// 取amount, 如果新bid_price超过, 要判断是否溢出
    pub fn collateral_amount(&self, last_bid_price: Balance, new_bid_price: Balance) -> Balance {
        if self.in_reverse_stage(new_bid_price) && new_bid_price > last_bid_price {
            Rate::checked_from_rational(sp_std::cmp::max(last_bid_price, self.target), new_bid_price)
                .and_then(|n| n.checked_mul_int(self.amount))
                .unwrap_or(self.amount)
        } else {
            self.amount
        }
    }
}

#[cfg_attr(feature = "std", derive(PartialEq, Eq))]
#[derive(Encode, Decode, Clone, RuntimeDebug)]
pub struct DebitAuctionItem<BlockNumber> {
    #[codec(compact)]
    pub initial_amount: Balance,
    #[codec(compact)]
    pub amount: Balance,
    #[codec(compact)]
    pub fix: Balance,
    /// Auction start time
    pub start_time: BlockNumber,
}

impl<BlockNumber> DebitAuctionItem<BlockNumber> {
    /// 取amount, 如果新bid_price超过, 要判断是否溢出
    pub fn amount_for_sale(&self, last_bid_price: Balance, new_bid_price: Balance) -> Balance {
        if new_bid_price > last_bid_price && new_bid_price > self.fix {
            Rate::checked_from_rational(sp_std::cmp::max(last_bid_price, self.fix), new_bid_price)
                .and_then(|n| n.checked_mul_int(self.amount))
                .unwrap_or(self.amount)
        } else {
            self.amount
        }
    }
}

#[cfg_attr(feature = "std", derive(PartialEq, Eq))]
#[derive(Encode, Decode, Clone, RuntimeDebug)]
pub struct SurplusAuctionItem<BlockNumber> {
    #[codec(compact)]
    pub amount: Balance,
    pub start_time: BlockNumber,
}