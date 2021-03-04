use sp_runtime::{DispatchError, DispatchResult};

pub trait RiskManager<AccountId, CurrencyId, Balance, DebitBalance> {
    fn get_bad_debt_value(currency_id: CurrencyId, debit_balance: DebitBalance) -> Balance;

    fn check_position_valid(
        currency_id: CurrencyId,
        collateral_balance: Balance,
        debit_balance: DebitBalance,
    ) -> DispatchResult;

    fn check_debit_cap(currency_id: CurrencyId, total_debit_balance: DebitBalance) -> DispatchResult;
}