#![cfg_attr(not(feature = "std"), no_std)]
use frame_support::{pallet_prelude::*, transactional};
use frame_system::pallet_prelude::*;
use orml_traits::{MultiCurrency, MultiCurrencyExtended};
use primitives::{Balance, CurrencyId};
use sp_runtime::{
	traits::{AccountIdConversion, One, Zero},
	DispatchError, DispatchResult, FixedPointNumber, ModuleId,
};

use primitives::Ratio;
use primitives::auction::AuctionManager;
use primitives::treasury::{CDPTreasury, CDPTreasuryExtended};

pub use mypallet::*;
mod default_weight;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub trait WeightInfo {
	fn auction_surplus() -> Weight;
	fn auction_debit() -> Weight;
	fn auction_collateral() -> Weight;
	fn set_collateral_auction_maximum_size() -> Weight;
}

#[frame_support::pallet]
pub mod mypallet {
	use super::*;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		type UpdateOrigin: EnsureOrigin<Self::Origin>;

		type Currency: MultiCurrencyExtended<Self::AccountId, CurrencyId = CurrencyId, Balance = Balance>;

		type AuctionManager: AuctionManager<Self::AccountId, CurrencyId = CurrencyId, Balance = Balance>;

		#[pallet::constant]
		type GetStableCurrencyId: Get<CurrencyId>;

		#[pallet::constant]
		type MaxAuctionsCount: Get<u32>;

		// type DEX: DEXManager<Self::AccountId, CurrencyId, Balance>;

		#[pallet::constant]
		type ModuleId: Get<ModuleId>;

		type WeightInfo: WeightInfo;
	}

	#[pallet::error]
	pub enum Error<T> {
		CollateralNotEnough,
		DebitPoolOverflow,
		SurplusPoolNotEnough,
		DebitPoolNotEnough,
	}

	#[pallet::event]
	#[pallet::generate_deposit(fn deposit_event)]
	pub enum Event<T: Config> {
		CollateralAuctionMaximumSizeUpdated(CurrencyId, Balance),
	}

	#[pallet::genesis_config]
	pub struct GenesisConfig {
		pub collateral_auction_maximum_size: Vec<(CurrencyId, Balance)>,
	}

	#[cfg(feature = "std")]
	impl Default for GenesisConfig {
		fn default() -> Self {
			GenesisConfig {
				collateral_auction_maximum_size: vec![],
			}
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> GenesisBuild<T> for GenesisConfig {
		fn build(&self) {
			self.collateral_auction_maximum_size
				.iter()
				.for_each(|(currency_id, size)| {
					CollateralAuctionMaximumSize::<T>::insert(currency_id, size);
				});
		}
	}

	#[cfg(feature = "std")]
	impl GenesisConfig {
		pub fn build_storage<T: Config>(&self) -> Result<sp_runtime::Storage, String> {
			<Self as GenesisBuild<T>>::build_storage(self)
		}

		pub fn assimilate_storage<T: Config>(&self, storage: &mut sp_runtime::Storage) -> Result<(), String> {
			<Self as GenesisBuild<T>>::assimilate_storage(self, storage)
		}
	}

	#[pallet::storage]
	#[pallet::getter(fn collateral_auction_maximum_size)]
	pub type CollateralAuctionMaximumSize<T: Config> = StorageMap<_, Twox64Concat, CurrencyId, Balance, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn debit_pool)]
	pub type DebitPool<T: Config> = StorageValue<_, Balance, ValueQuery>;

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_finalize(_now: T::BlockNumber) {
			// offset the same amount between debit pool and surplus pool
			Self::offset_surplus_and_debit();
		}
	}

	#[pallet::call]
	impl<T:Config> Pallet<T> {
		#[pallet::weight(T::WeightInfo::auction_surplus())]
		#[transactional]
		pub fn auction_surplus(origin: OriginFor<T>, amount: Balance) -> DispatchResultWithPostInfo {
			T::UpdateOrigin::ensure_origin(origin)?;
			ensure!(
				Self::surplus_pool().saturating_sub(T::AuctionManager::get_total_surplus_in_auction()) >= amount,
				Error::<T>::SurplusPoolNotEnough,
			);
			T::AuctionManager::new_surplus_auction(amount)?;
			Ok(().into())
		}

		#[pallet::weight(T::WeightInfo::auction_debit())]
		#[transactional]
		pub fn auction_debit(
			origin: OriginFor<T>,
			amount: Balance,
			initial_price: Balance,
		) -> DispatchResultWithPostInfo {
			T::UpdateOrigin::ensure_origin(origin)?;
			ensure!(
				Self::debit_pool().saturating_sub(T::AuctionManager::get_total_debit_in_auction()) >= amount,
				Error::<T>::DebitPoolNotEnough,
			);
			T::AuctionManager::new_debit_auction(amount, initial_price)?;
			Ok(().into())
		}

		#[pallet::weight(T::WeightInfo::auction_collateral())]
		#[transactional]
		pub fn auction_collateral(
			origin: OriginFor<T>,
			currency_id: CurrencyId,
			amount: Balance,
			target: Balance,
			splited: bool,
		) -> DispatchResultWithPostInfo {
			T::UpdateOrigin::ensure_origin(origin)?;
			<Self as CDPTreasuryExtended<T::AccountId>>::create_collateral_auctions(
				currency_id,
				amount,
				target,
				Self::account_id(),
				splited,
			)?;
			Ok(().into())
		}


		#[pallet::weight((T::WeightInfo::set_collateral_auction_maximum_size(), DispatchClass::Operational))]
		#[transactional]
		pub fn set_collateral_auction_maximum_size(
			origin: OriginFor<T>,
			currency_id: CurrencyId,
			size: Balance,
		) -> DispatchResultWithPostInfo {
			T::UpdateOrigin::ensure_origin(origin)?;
			CollateralAuctionMaximumSize::<T>::insert(currency_id, size);
			Self::deposit_event(Event::CollateralAuctionMaximumSizeUpdated(currency_id, size));
			Ok(().into())
		}
	}
}

impl<T: Config> Pallet<T> {
	/// Get account of cdp treasury module.
	pub fn account_id() -> T::AccountId {
		T::ModuleId::get().into_account()
	}

	/// 剩余池有用户的稳定资产
	pub fn surplus_pool() -> Balance {
		T::Currency::free_balance(T::GetStableCurrencyId::get(), &Self::account_id())
	}

	/// 所有抵押品
	pub fn total_collaterals(currency_id: CurrencyId) -> Balance {
		T::Currency::free_balance(currency_id, &Self::account_id())
	}

	/// 不在拍卖的抵押品
	pub fn total_collaterals_not_in_auction(currency_id: CurrencyId) -> Balance {
		T::Currency::free_balance(currency_id, &Self::account_id())
			.saturating_sub(T::AuctionManager::get_total_collateral_in_auction(currency_id))
	}

	/// 解决debit和suplus价值偏差,从treasure消除
	fn offset_surplus_and_debit() {
		let offset_amount = sp_std::cmp::min(Self::debit_pool(), Self::surplus_pool());

		// Burn the amount that is equal to offset amount of stable currency.
		if !offset_amount.is_zero()
			&& T::Currency::withdraw(T::GetStableCurrencyId::get(), &Self::account_id(), offset_amount).is_ok()
		{
			DebitPool::<T>::mutate(|debit| {
				*debit = debit
					.checked_sub(offset_amount)
					.expect("offset = min(debit, surplus); qed")
			});
		}
	}
}

impl<T: Config> CDPTreasury<T::AccountId> for Pallet<T> {
	type Balance = Balance;
	type CurrencyId = CurrencyId;

	// get方法
	fn get_debit_pool() -> Self::Balance {
		Self::debit_pool()
	}
	fn get_surplus_pool() -> Self::Balance {
		Self::surplus_pool()
	}
	fn get_total_collaterals(id: Self::CurrencyId) -> Self::Balance {
		Self::total_collaterals(id)
	}
	// debit占比
	fn get_debit_proportion(amount: Self::Balance) -> Ratio {
		let stable_total_supply = T::Currency::total_issuance(T::GetStableCurrencyId::get());
		Ratio::checked_from_rational(amount, stable_total_supply).unwrap_or_default()
	}
	/// 增加DebitPool
	fn on_system_debit(amount: Self::Balance) -> DispatchResult {
		DebitPool::<T>::try_mutate(|debit_pool| -> DispatchResult {
			*debit_pool = debit_pool.checked_add(amount).ok_or(Error::<T>::DebitPoolOverflow)?;
			Ok(())
		})
	}
	/// 增发debit到treasury
	fn on_system_surplus(amount: Self::Balance) -> DispatchResult {
		Self::issue_debit(&Self::account_id(), amount, true)
	}
	/// 发stablecoin
	fn issue_debit(who: &T::AccountId, debit: Self::Balance, backed: bool) -> DispatchResult {
		// increase system debit if the debit is unbacked
		if !backed {
			Self::on_system_debit(debit)?;
		}
		T::Currency::deposit(T::GetStableCurrencyId::get(), who, debit)?;

		Ok(())
	}
	/// 消除stablecoin
	fn burn_debit(who: &T::AccountId, debit: Self::Balance) -> DispatchResult {
		T::Currency::withdraw(T::GetStableCurrencyId::get(), who, debit)
	}
	/// 从个人到treasury
	fn deposit_surplus(from: &T::AccountId, surplus: Self::Balance) -> DispatchResult {
		T::Currency::transfer(T::GetStableCurrencyId::get(), from, &Self::account_id(), surplus)
	}
	/// 转移抵押品, 罚款loan->treasury
	fn deposit_collateral(from: &T::AccountId, currency_id: Self::CurrencyId, amount: Self::Balance) -> DispatchResult {
		T::Currency::transfer(currency_id, from, &Self::account_id(), amount)
	}
	/// 抵押品转出到账户里
	fn withdraw_collateral(to: &T::AccountId, currency_id: Self::CurrencyId, amount: Self::Balance) -> DispatchResult {
		T::Currency::transfer(currency_id, &Self::account_id(), to, amount)
	}
}

impl<T: Config> CDPTreasuryExtended<T::AccountId> for Pallet<T> {

	fn swap_exact_collateral_in_auction_to_stable(
		currency_id: CurrencyId,
		supply_amount: Balance,
		min_target_amount: Balance,
		price_impact_limit: Option<Ratio>,
	) -> sp_std::result::Result<Balance, DispatchError> {
		ensure!(
			Self::total_collaterals(currency_id) >= supply_amount
				&& T::AuctionManager::get_total_collateral_in_auction(currency_id) >= supply_amount,
			Error::<T>::CollateralNotEnough,
		);
		let amount :Balance = 0;
		Ok(amount)
		// TODO: exchange
		// T::DEX::swap_with_exact_supply(
		// 	&Self::account_id(),
		// 	&[currency_id, T::GetStableCurrencyId::get()],
		// 	supply_amount,
		// 	min_target_amount,
		// 	price_impact_limit,
		// )
	}


	fn swap_collateral_not_in_auction_with_exact_stable(
		currency_id: CurrencyId,
		target_amount: Balance,
		max_supply_amount: Balance,
		price_impact_limit: Option<Ratio>,
	) -> sp_std::result::Result<Balance, DispatchError> {
		ensure!(
			Self::total_collaterals_not_in_auction(currency_id) >= max_supply_amount,
			Error::<T>::CollateralNotEnough,
		);

		let amount :Balance = 0;
		Ok(amount)
		// TODO: exchange
		// T::DEX::swap_with_exact_target(
		// 	&Self::account_id(),
		// 	&[currency_id, T::GetStableCurrencyId::get()],
		// 	target_amount,
		// 	max_supply_amount,
		// 	price_impact_limit,
		// )
	}

	/// 启动拍卖
	fn create_collateral_auctions(
		currency_id: CurrencyId,
		amount: Balance,
		target: Balance,
		refund_receiver: T::AccountId,
		splited: bool,
	) -> DispatchResult {
		ensure!(
			Self::total_collaterals_not_in_auction(currency_id) >= amount,
			Error::<T>::CollateralNotEnough,
		);

		let mut unhandled_collateral_amount = amount;
		let mut unhandled_target = target;
		let collateral_auction_maximum_size = Self::collateral_auction_maximum_size(currency_id);
		let max_auctions_count: Balance = T::MaxAuctionsCount::get().into();
		let lots_count = if !splited
			|| max_auctions_count.is_zero()
			|| collateral_auction_maximum_size.is_zero()
			|| amount <= collateral_auction_maximum_size
		{
			One::one()
		} else {
			let mut count = amount
				.checked_div(collateral_auction_maximum_size)
				.expect("collateral auction maximum size is not zero; qed");

			let remainder = amount
				.checked_rem(collateral_auction_maximum_size)
				.expect("collateral auction maximum size is not zero; qed");
			if !remainder.is_zero() {
				count = count.saturating_add(One::one());
			}
			sp_std::cmp::min(count, max_auctions_count)
		};
		let average_amount_per_lot = amount.checked_div(lots_count).expect("lots count is at least 1; qed");
		let average_target_per_lot = target.checked_div(lots_count).expect("lots count is at least 1; qed");
		let mut created_lots: Balance = Zero::zero();

		while !unhandled_collateral_amount.is_zero() {
			created_lots = created_lots.saturating_add(One::one());
			let (lot_collateral_amount, lot_target) = if created_lots == lots_count {
				// the last lot may be have some remnant than average
				(unhandled_collateral_amount, unhandled_target)
			} else {
				(average_amount_per_lot, average_target_per_lot)
			};

			T::AuctionManager::new_collateral_auction(
				&refund_receiver,
				currency_id,
				lot_collateral_amount,
				lot_target,
			)?;

			unhandled_collateral_amount = unhandled_collateral_amount.saturating_sub(lot_collateral_amount);
			unhandled_target = unhandled_target.saturating_sub(lot_target);
		}
		Ok(())
	}
}


