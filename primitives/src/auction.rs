use sp_std::fmt::Debug;
use codec::FullCodec;
use sp_runtime::DispatchResult;



pub trait AuctionManager<AccountId> {
    type CurrencyId;
    type Balance;
    type AuctionId: FullCodec + Debug + Clone + Eq + PartialEq;

    fn new_collateral_auction(
        refund_recipient: &AccountId,
        currency_id: Self::CurrencyId,
        amount: Self::Balance,
        target: Self::Balance,
    ) -> DispatchResult;
    fn new_debit_auction(amount: Self::Balance, fix: Self::Balance) -> DispatchResult;
    fn new_surplus_auction(amount: Self::Balance) -> DispatchResult;
    fn cancel_auction(id: Self::AuctionId) -> DispatchResult;

    fn get_total_collateral_in_auction(id: Self::CurrencyId) -> Self::Balance;
    fn get_total_surplus_in_auction() -> Self::Balance;
    fn get_total_debit_in_auction() -> Self::Balance;
    fn get_total_target_in_auction() -> Self::Balance;
}