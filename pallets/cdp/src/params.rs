use codec::{ Decode, Encode };
use sp_runtime::RuntimeDebug;
use primitives::{Rate, Ratio, Balance};

/// Risk management params
#[derive(Encode, Decode, Clone, RuntimeDebug, PartialEq, Eq, Default)]
pub struct RiskManagementParams {
    /// Maximum total debit value generated from it, when reach the hard
    /// cap, CDP's owner cannot issue more stablecoin under the collateral
    /// type.
    pub maximum_total_debit_value: Balance,

    /// Extra stability fee rate, `None` value means not set
    pub stability_fee: Option<Rate>,

    /// Liquidation ratio, when the collateral ratio of
    /// CDP under this collateral type is below the liquidation ratio, this
    /// CDP is unsafe and can be liquidated. `None` value means not set
    pub liquidation_ratio: Option<Ratio>,

    /// Liquidation penalty rate, when liquidation occurs,
    /// CDP will be deducted an additional penalty base on the product of
    /// penalty rate and debit value. `None` value means not set
    pub liquidation_penalty: Option<Rate>,

    /// Required collateral ratio, if it's set, cannot adjust the position
    /// of CDP so that the current collateral ratio is lower than the
    /// required collateral ratio. `None` value means not set
    pub required_collateral_ratio: Option<Ratio>,
}


/// Liquidation strategy available
#[derive(Encode, Decode, Clone, RuntimeDebug, PartialEq, Eq)]
pub enum LiquidationStrategy {
    /// Liquidation CDP's collateral by create collateral auction
    Auction,
    /// Liquidation CDP's collateral by swap with DEX
    Exchange,
}

#[derive(Encode, Decode, Eq, PartialEq, Copy, Clone, RuntimeDebug, Default)]
pub struct Position {
    /// The amount of collateral.
    pub collateral: Balance,
    /// The amount of debit.
    pub debit: Balance,
}