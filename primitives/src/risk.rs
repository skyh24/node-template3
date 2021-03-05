use sp_runtime::{DispatchError, DispatchResult};

pub trait RiskManager<AccountId> {
    type CurrencyId;
    type Balance;
    type DebitBalance;

    fn get_bad_debt_value(currency_id: Self::CurrencyId, debit_balance: Self::DebitBalance) -> Self::Balance;

    fn check_position_valid(
        currency_id: Self::CurrencyId,
        collateral_balance: Self::Balance,
        debit_balance: Self::DebitBalance,
    ) -> DispatchResult;

    fn check_debit_cap(currency_id: Self::CurrencyId, total_debit_balance: Self::DebitBalance) -> DispatchResult;
}