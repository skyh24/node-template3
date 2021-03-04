#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::{pallet_prelude::*, transactional};
use frame_system::pallet_prelude::*;
use sp_runtime::traits::{CheckedDiv, CheckedMul};

use orml_traits::{DataProvider};
use primitives::{CurrencyId, Price};
use primitives::prices::PriceProvider;

pub use mypallet::*;
mod default_weight;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

pub trait WeightInfo {
	fn lock_price() -> Weight;
	fn unlock_price() -> Weight;
}

#[frame_support::pallet]
pub mod mypallet {
	use super::*;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		/// The data source, such as Oracle.
		type Source: DataProvider<CurrencyId, Price>;

		#[pallet::constant]
		type GetStableCurrencyId: Get<CurrencyId>;

		#[pallet::constant]
		type StableCurrencyFixedPrice: Get<Price>;

		#[pallet::constant]
		type GetStakingCurrencyId: Get<CurrencyId>;

		// #[pallet::constant]
		// type GetLiquidCurrencyId: Get<CurrencyId>;
		// type LiquidStakingExchangeRateProvider: ExchangeRateProvider;

		/// The origin which may lock and unlock prices feed to system.
		type LockOrigin: EnsureOrigin<Self::Origin>;

		/// Weight information for the extrinsics in this module.
		type WeightInfo: WeightInfo;
	}

	#[pallet::storage]
	#[pallet::getter(fn locked_price)]
	pub type LockedPrice<T: Config> = StorageMap<_, Twox64Concat, CurrencyId, Price, OptionQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(crate) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Lock price. \[currency_id, locked_price\]
		LockPrice(CurrencyId, Price),
		/// Unlock price. \[currency_id\]
		UnlockPrice(CurrencyId),
	}
	
	// Errors inform users that something went wrong.
	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
		StorageOverflow,
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {

	}

	#[pallet::call]
	impl<T:Config> Pallet<T> {
		#[pallet::weight((T::WeightInfo::lock_price(), DispatchClass::Operational))]
		#[transactional]
		pub fn lock_price(origin: OriginFor<T>, currency_id: CurrencyId) -> DispatchResultWithPostInfo {
			T::LockOrigin::ensure_origin(origin)?;
			<Pallet<T> as PriceProvider<CurrencyId>>::lock_price(currency_id);
			Ok(().into())
		}

		#[pallet::weight((T::WeightInfo::unlock_price(), DispatchClass::Operational))]
		#[transactional]
		pub fn unlock_price(origin: OriginFor<T>, currency_id: CurrencyId) -> DispatchResultWithPostInfo {
			T::LockOrigin::ensure_origin(origin)?;
			<Pallet<T> as PriceProvider<CurrencyId>>::unlock_price(currency_id);
			Ok(().into())
		}
	}
}

impl<T: Config> PriceProvider<CurrencyId> for Pallet<T> {
	/// get relative price between two currency types
	fn get_relative_price(base_currency_id: CurrencyId, quote_currency_id: CurrencyId) -> Option<Price> {
		if let (Some(base_price), Some(quote_price)) =
		(Self::get_price(base_currency_id), Self::get_price(quote_currency_id))
		{
			base_price.checked_div(&quote_price)
		} else {
			None
		}
	}

	/// 取出价格:
	/// 1. 稳定资产1
	/// 2. dex资产从流动性获取
	/// 3. 外币资产从oracle获取
	fn get_price(currency_id: CurrencyId) -> Option<Price> {
		if currency_id == T::GetStableCurrencyId::get() {
			// if is stable currency, return fixed price
			Some(T::StableCurrencyFixedPrice::get())
		// } else if currency_id == T::GetLiquidCurrencyId::get() {
		// 	// if is liquid currency, return the product of staking currency price and
		// 	// liquid/staking exchange rate.
		// 	Self::get_price(T::GetStakingCurrencyId::get())
		// 		.and_then(|n| n.checked_mul(&T::LiquidStakingExchangeRateProvider::get_exchange_rate()))
		} else {
			// if locked price exists, return it, otherwise return latest price from oracle.
			Self::locked_price(currency_id).or_else(|| T::Source::get(&currency_id))
		}
	}

	fn lock_price(currency_id: CurrencyId) {
		// lock price when get valid price from source
		if let Some(val) = T::Source::get(&currency_id) {
			LockedPrice::<T>::insert(currency_id, val);
			<Pallet<T>>::deposit_event(Event::LockPrice(currency_id, val));
		}
	}

	fn unlock_price(currency_id: CurrencyId) {
		LockedPrice::<T>::remove(currency_id);
		<Pallet<T>>::deposit_event(Event::UnlockPrice(currency_id));
	}
}
