use sp_runtime::{DispatchError, DispatchResult};

use crate::{Rate, Ratio};

/// An abstraction of cdp treasury for Honzon Protocol.
pub trait CDPTreasury<AccountId> {
    type Balance;
    type CurrencyId;

    /// get surplus amount of cdp treasury
    fn get_surplus_pool() -> Self::Balance;

    /// get debit amount of cdp treasury
    fn get_debit_pool() -> Self::Balance;

    /// get collateral assets amount of cdp treasury
    fn get_total_collaterals(id: Self::CurrencyId) -> Self::Balance;

    /// calculate the proportion of specific debit amount for the whole system
    fn get_debit_proportion(amount: Self::Balance) -> Ratio;

    /// issue debit for cdp treasury
    fn on_system_debit(amount: Self::Balance) -> DispatchResult;

    /// issue surplus(stable currency) for cdp treasury
    fn on_system_surplus(amount: Self::Balance) -> DispatchResult;

    /// issue debit to `who`
    /// if backed flag is true, means the debit to issue is backed on some
    /// assets, otherwise will increase same amount of debit to system debit.
    fn issue_debit(who: &AccountId, debit: Self::Balance, backed: bool) -> DispatchResult;

    /// burn debit(stable currency) of `who`
    fn burn_debit(who: &AccountId, debit: Self::Balance) -> DispatchResult;

    /// deposit surplus(stable currency) to cdp treasury by `from`
    fn deposit_surplus(from: &AccountId, surplus: Self::Balance) -> DispatchResult;

    /// deposit collateral assets to cdp treasury by `who`
    fn deposit_collateral(from: &AccountId, currency_id: Self::CurrencyId, amount: Self::Balance) -> DispatchResult;

    /// withdraw collateral assets of cdp treasury to `who`
    fn withdraw_collateral(to: &AccountId, currency_id: Self::CurrencyId, amount: Self::Balance) -> DispatchResult;
}

pub trait CDPTreasuryExtended<AccountId>: CDPTreasury<AccountId> {
    fn swap_exact_collateral_in_auction_to_stable(
        currency_id: Self::CurrencyId,
        supply_amount: Self::Balance,
        min_target_amount: Self::Balance,
        price_impact_limit: Option<Ratio>,
    ) -> sp_std::result::Result<Self::Balance, DispatchError>;

    fn swap_collateral_not_in_auction_with_exact_stable(
        currency_id: Self::CurrencyId,
        target_amount: Self::Balance,
        max_supply_amount: Self::Balance,
        price_impact_limit: Option<Ratio>,
    ) -> sp_std::result::Result<Self::Balance, DispatchError>;

    fn create_collateral_auctions(
        currency_id: Self::CurrencyId,
        amount: Self::Balance,
        target: Self::Balance,
        refund_receiver: AccountId,
        splited: bool,
    ) -> DispatchResult;
}