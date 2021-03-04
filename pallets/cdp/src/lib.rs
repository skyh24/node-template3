#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::{pallet_prelude::*, transactional};
use frame_system::pallet_prelude::*;
use sp_runtime::{
	traits::{BlakeTwo256, Bounded, Convert, Hash, Saturating, StaticLookup, Zero},
	transaction_validity::{
		InvalidTransaction, TransactionPriority, TransactionSource, TransactionValidity, ValidTransaction,
	},
	DispatchError, DispatchResult, FixedPointNumber, RandomNumberGenerator, RuntimeDebug,
};

use loans::Position;
use orml_traits::Change;
use primitives::{Rate, Ratio, ExchangeRate, Price, Amount, Balance, CurrencyId};
use primitives::risk::RiskManager;
use primitives::prices::PriceProvider;
use primitives::treasury::{CDPTreasury, CDPTreasuryExtended};

pub mod params;
pub use params::*;

pub use mypallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub trait WeightInfo {
	fn set_collateral_params() -> Weight;
	fn set_global_params() -> Weight;
	fn liquidate_by_auction() -> Weight;
	fn liquidate_by_dex() -> Weight;
	fn settle() -> Weight;
}

pub const OFFCHAIN_WORKER_DATA: &[u8] = b"bandot/cdp/data/";
pub const OFFCHAIN_WORKER_LOCK: &[u8] = b"bandot/cdp/lock/";
pub const OFFCHAIN_WORKER_MAX_ITERATIONS: &[u8] = b"bandot/cdp/max-iterations/";
pub const LOCK_DURATION: u64 = 100;
pub const DEFAULT_MAX_ITERATIONS: u32 = 1000;

pub type LoansOf<T> = loans::Module<T>;

// typedef to help polkadot.js disambiguate Change with different generic
// parameters
type ChangeOptionRate = Change<Option<Rate>>;
type ChangeOptionRatio = Change<Option<Ratio>>;
type ChangeBalance = Change<Balance>;

pub struct DebitExchangeRateConvertor<T>(sp_std::marker::PhantomData<T>);

impl<T> Convert<(CurrencyId, Balance), Balance> for DebitExchangeRateConvertor<T>
	where T: Config,
{
	fn convert((currency_id, balance): (CurrencyId, Balance)) -> Balance {
		<Module<T>>::get_debit_exchange_rate(currency_id).saturating_mul_int(balance)
	}
}

#[frame_support::pallet]
pub mod mypallet {
	use super::*;

	#[pallet::config]
	pub trait Config: frame_system::Config  + loans::Config {
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		type UpdateOrigin: EnsureOrigin<Self::Origin>;

		#[pallet::constant]
		type GetStableCurrencyId: Get<CurrencyId>;
		#[pallet::constant]
		type CollateralCurrencyIds: Get<Vec<CurrencyId>>;

		#[pallet::constant]
		type DefaultLiquidationRatio: Get<Ratio>;
		#[pallet::constant]
		type DefaultDebitExchangeRate: Get<ExchangeRate>;

		#[pallet::constant]
		type DefaultLiquidationPenalty: Get<Rate>;
		#[pallet::constant]
		type MinimumDebitValue: Get<Balance>;

		#[pallet::constant]
		type MaxSlippageSwapWithDEX: Get<Ratio>;

		type CDPTreasury: CDPTreasuryExtended<Self::AccountId, Balance = Balance, CurrencyId = CurrencyId>;

		type PriceSource: PriceProvider<CurrencyId>;

		//type DEX: DEXManager<Self::AccountId, CurrencyId, Balance>;
		//type EmergencyShutdown: EmergencyShutdown;

		#[pallet::constant]
		type UnsignedPriority: Get<TransactionPriority>;

		type WeightInfo: WeightInfo;
	}

	#[pallet::error]
	pub enum Error<T> {
		ExceedDebitValueHardCap,
		BelowRequiredCollateralRatio,
		BelowLiquidationRatio,
		MustBeUnsafe,
		InvalidCollateralType,
		RemainDebitValueTooSmall,
		InvalidFeedPrice,
		NoDebitValue,
		AlreadyShutdown,
		MustAfterShutdown,
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(crate) fn deposit_event)]
	pub enum Event<T: Config> {
		LiquidateUnsafeCDP(CurrencyId, T::AccountId, Balance, Balance, LiquidationStrategy),
		SettleCDPInDebit(CurrencyId, T::AccountId),
		StabilityFeeUpdated(CurrencyId, Option<Rate>),
		LiquidationRatioUpdated(CurrencyId, Option<Ratio>),
		LiquidationPenaltyUpdated(CurrencyId, Option<Rate>),
		RequiredCollateralRatioUpdated(CurrencyId, Option<Ratio>),
		MaximumTotalDebitValueUpdated(CurrencyId, Balance),
		GlobalStabilityFeeUpdated(Rate),
	}

	#[pallet::storage]
	#[pallet::getter(fn debit_exchange_rate)]
	pub type DebitExchangeRate<T: Config> = StorageMap<_, Twox64Concat, CurrencyId, ExchangeRate, OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn global_stability_fee)]
	pub type GlobalStabilityFee<T: Config> = StorageValue<_, Rate, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn collateral_params)]
	pub type CollateralParams<T: Config> = StorageMap<_, Twox64Concat, CurrencyId, RiskManagementParams, ValueQuery>;

	#[pallet::genesis_config]
	pub struct GenesisConfig {
		#[allow(clippy::type_complexity)]
		pub collaterals_params: Vec<(
			CurrencyId,
			Option<Rate>,
			Option<Ratio>,
			Option<Rate>,
			Option<Ratio>,
			Balance,
		)>,
		pub global_stability_fee: Rate,
	}

	#[cfg(feature = "std")]
	impl Default for GenesisConfig {
		fn default() -> Self {
			GenesisConfig {
				collaterals_params: vec![],
				global_stability_fee: Default::default(),
			}
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> GenesisBuild<T> for GenesisConfig {
		fn build(&self) {
			self.collaterals_params.iter().for_each(
				|(
					 currency_id,
					 stability_fee,
					 liquidation_ratio,
					 liquidation_penalty,
					 required_collateral_ratio,
					 maximum_total_debit_value,
				 )| {
					CollateralParams::<T>::insert(
						currency_id,
						RiskManagementParams {
							maximum_total_debit_value: *maximum_total_debit_value,
							stability_fee: *stability_fee,
							liquidation_ratio: *liquidation_ratio,
							liquidation_penalty: *liquidation_penalty,
							required_collateral_ratio: *required_collateral_ratio,
						},
					);
				},
			);
			GlobalStabilityFee::<T>::put(self.global_stability_fee);
		}
	}

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		// fn on_finalize(_now: T::BlockNumber) {
		// 	// collect stability fee for all types of collateral

		// /// submit unsigned tx to trigger liquidation or settlement.
		// fn offchain_worker(now: T::BlockNumber) {
	}

	#[pallet::call]
	impl<T:Config> Pallet<T> {
		#[pallet::weight(T::WeightInfo::liquidate_by_dex())]
		#[transactional]
		pub fn liquidate(
			origin: OriginFor<T>,
			currency_id: CurrencyId,
			who: <T::Lookup as StaticLookup>::Source,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;
			let who = T::Lookup::lookup(who)?;
			//ensure!(!T::EmergencyShutdown::is_shutdown(), Error::<T>::AlreadyShutdown);
			Self::liquidate_unsafe_cdp(who, currency_id)?;
			Ok(().into())
		}

		#[pallet::weight(T::WeightInfo::settle())]
		#[transactional]
		pub fn settle(
			origin: OriginFor<T>,
			currency_id: CurrencyId,
			who: <T::Lookup as StaticLookup>::Source,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;
			let who = T::Lookup::lookup(who)?;
			//ensure!(T::EmergencyShutdown::is_shutdown(), Error::<T>::MustAfterShutdown);
			Self::settle_cdp_has_debit(who, currency_id)?;
			Ok(().into())
		}

		#[pallet::weight((T::WeightInfo::set_global_params(), DispatchClass::Operational))]
		#[transactional]
		pub fn set_global_params(origin: OriginFor<T>, global_stability_fee: Rate) -> DispatchResultWithPostInfo {
			T::UpdateOrigin::ensure_origin(origin)?;
			GlobalStabilityFee::<T>::put(global_stability_fee);
			Self::deposit_event(Event::GlobalStabilityFeeUpdated(global_stability_fee));
			Ok(().into())
		}

		#[pallet::weight((T::WeightInfo::set_collateral_params(), DispatchClass::Operational))]
		#[transactional]
		pub fn set_collateral_params(
			origin: OriginFor<T>,
			currency_id: CurrencyId,
			stability_fee: ChangeOptionRate,
			liquidation_ratio: ChangeOptionRatio,
			liquidation_penalty: ChangeOptionRate,
			required_collateral_ratio: ChangeOptionRatio,
			maximum_total_debit_value: ChangeBalance,
		) -> DispatchResultWithPostInfo {
			T::UpdateOrigin::ensure_origin(origin)?;
			ensure!(
				T::CollateralCurrencyIds::get().contains(&currency_id),
				Error::<T>::InvalidCollateralType,
			);

			let mut collateral_params = Self::collateral_params(currency_id);
			if let Change::NewValue(update) = stability_fee {
				collateral_params.stability_fee = update;
				Self::deposit_event(Event::StabilityFeeUpdated(currency_id, update));
			}
			if let Change::NewValue(update) = liquidation_ratio {
				collateral_params.liquidation_ratio = update;
				Self::deposit_event(Event::LiquidationRatioUpdated(currency_id, update));
			}
			if let Change::NewValue(update) = liquidation_penalty {
				collateral_params.liquidation_penalty = update;
				Self::deposit_event(Event::LiquidationPenaltyUpdated(currency_id, update));
			}
			if let Change::NewValue(update) = required_collateral_ratio {
				collateral_params.required_collateral_ratio = update;
				Self::deposit_event(Event::RequiredCollateralRatioUpdated(currency_id, update));
			}
			if let Change::NewValue(val) = maximum_total_debit_value {
				collateral_params.maximum_total_debit_value = val;
				Self::deposit_event(Event::MaximumTotalDebitValueUpdated(currency_id, val));
			}
			CollateralParams::<T>::insert(currency_id, collateral_params);
			Ok(().into())
		}
	}

	// #[pallet::validate_unsigned]
	// impl<T: Config> ValidateUnsigned for Pallet<T> {
	// }
}

impl<T: Config> Pallet<T> {
	// fn submit_unsigned_liquidation_tx(currency_id: CurrencyId, who: T::AccountId) {
	// 	let who = T::Lookup::unlookup(who);
	// 	let call = Call::<T>::liquidate(currency_id, who.clone());
	// 	if SubmitTransaction::<T, Call<T>>::submit_unsigned_transaction(call.into()).is_err() {
	// 		debug::info!(
	// 			target: "cdp-engine offchain worker",
	// 			"submit unsigned liquidation tx for \nCDP - AccountId {:?} CurrencyId {:?} \nfailed!",
	// 			who, currency_id,
	// 		);
	// 	}
	// }
	//
	// fn submit_unsigned_settlement_tx(currency_id: CurrencyId, who: T::AccountId) {
	// 	let who = T::Lookup::unlookup(who);
	// 	let call = Call::<T>::settle(currency_id, who.clone());
	// 	if SubmitTransaction::<T, Call<T>>::submit_unsigned_transaction(call.into()).is_err() {
	// 		debug::info!(
	// 			target: "cdp-engine offchain worker",
	// 			"submit unsigned settlement tx for \nCDP - AccountId {:?} CurrencyId {:?} \nfailed!",
	// 			who, currency_id,
	// 		);
	// 	}
	// }

	pub fn is_cdp_unsafe(currency_id: CurrencyId, collateral: Balance, debit: Balance) -> bool {
		let stable_currency_id = T::GetStableCurrencyId::get();

		if let Some(feed_price) = T::PriceSource::get_relative_price(currency_id, stable_currency_id) {
			let collateral_ratio = Self::calculate_collateral_ratio(currency_id, collateral, debit, feed_price);
			collateral_ratio < Self::get_liquidation_ratio(currency_id)
		} else {
			false
		}
	}

	pub fn maximum_total_debit_value(currency_id: CurrencyId) -> Balance {
		Self::collateral_params(currency_id).maximum_total_debit_value
	}

	pub fn required_collateral_ratio(currency_id: CurrencyId) -> Option<Ratio> {
		Self::collateral_params(currency_id).required_collateral_ratio
	}

	pub fn get_stability_fee(currency_id: CurrencyId) -> Rate {
		Self::collateral_params(currency_id)
			.stability_fee
			.unwrap_or_default()
			.saturating_add(Self::global_stability_fee())
	}

	pub fn get_liquidation_ratio(currency_id: CurrencyId) -> Ratio {
		Self::collateral_params(currency_id)
			.liquidation_ratio
			.unwrap_or_else(T::DefaultLiquidationRatio::get)
	}

	pub fn get_liquidation_penalty(currency_id: CurrencyId) -> Rate {
		Self::collateral_params(currency_id)
			.liquidation_penalty
			.unwrap_or_else(T::DefaultLiquidationPenalty::get)
	}

	pub fn get_debit_exchange_rate(currency_id: CurrencyId) -> ExchangeRate {
		Self::debit_exchange_rate(currency_id).unwrap_or_else(T::DefaultDebitExchangeRate::get)
	}

	pub fn get_debit_value(currency_id: CurrencyId, debit_balance: Balance) -> Balance {
		crate::DebitExchangeRateConvertor::<T>::convert((currency_id, debit_balance))
	}

	pub fn calculate_collateral_ratio(
		currency_id: CurrencyId,
		collateral_balance: Balance,
		debit_balance: Balance,
		price: Price,
	) -> Ratio {
		let locked_collateral_value = price.saturating_mul_int(collateral_balance);
		let debit_value = Self::get_debit_value(currency_id, debit_balance);

		Ratio::checked_from_rational(locked_collateral_value, debit_value).unwrap_or_else(Rate::max_value)
	}

	// TODO:把loan合并
	pub fn adjust_position(
		who: &T::AccountId,
		currency_id: CurrencyId,
		collateral_adjustment: Amount,
		debit_adjustment: Amount,
	) -> DispatchResult {
		ensure!(
			T::CollateralCurrencyIds::get().contains(&currency_id),
			Error::<T>::InvalidCollateralType,
		);
		<LoansOf<T>>::adjust_position(who, currency_id, collateral_adjustment, debit_adjustment)?;
		Ok(())
	}

	// 解决坏账
	pub fn settle_cdp_has_debit(who: T::AccountId, currency_id: CurrencyId) -> DispatchResult {
		let Position { collateral, debit } = <LoansOf<T>>::positions(currency_id, &who);
		ensure!(!debit.is_zero(), Error::<T>::NoDebitValue);

		// confiscate collateral in cdp to cdp treasury
		// and decrease CDP's debit to zero
		let settle_price: Price = T::PriceSource::get_relative_price(T::GetStableCurrencyId::get(), currency_id)
			.ok_or(Error::<T>::InvalidFeedPrice)?;
		let bad_debt_value = Self::get_debit_value(currency_id, debit);
		let confiscate_collateral_amount =
			sp_std::cmp::min(settle_price.saturating_mul_int(bad_debt_value), collateral);

		// 没收所有抵押和债务
		<LoansOf<T>>::confiscate_collateral_and_debit(&who, currency_id, confiscate_collateral_amount, debit)?;

		Self::deposit_event(Event::SettleCDPInDebit(currency_id, who));
		Ok(())
	}

	// 清算CDP
	pub fn liquidate_unsafe_cdp(who: T::AccountId, currency_id: CurrencyId) -> DispatchResult {
		let Position { collateral, debit } = <LoansOf<T>>::positions(currency_id, &who);

		// ensure the cdp is unsafe
		ensure!(
			Self::is_cdp_unsafe(currency_id, collateral, debit),
			Error::<T>::MustBeUnsafe
		);

		// confiscate all collateral and debit of unsafe cdp to cdp treasury
		<LoansOf<T>>::confiscate_collateral_and_debit(&who, currency_id, collateral, debit)?;

		let bad_debt_value = Self::get_debit_value(currency_id, debit);
		let target_stable_amount = Self::get_liquidation_penalty(currency_id).saturating_mul_acc_int(bad_debt_value);

		// try use collateral to swap enough native token in DEX when the price impact
		// is below the limit, otherwise create collateral auctions.
		let liquidation_strategy = (|| -> Result<LiquidationStrategy, DispatchError> {
			// swap exact stable with DEX in limit of price impact
			if let Ok(actual_supply_collateral) =
			<T as Config>::CDPTreasury::swap_collateral_not_in_auction_with_exact_stable(
				currency_id,
				target_stable_amount,
				collateral,
				Some(T::MaxSlippageSwapWithDEX::get()),
			) {
				// refund remain collateral to CDP owner
				let refund_collateral_amount = collateral
					.checked_sub(actual_supply_collateral)
					.expect("swap succecced means collateral >= actual_supply_collateral; qed");

				<T as Config>::CDPTreasury::withdraw_collateral(&who, currency_id, refund_collateral_amount)?;

				return Ok(LiquidationStrategy::Exchange);
			}

			// create collateral auctions by cdp treasury
			<T as Config>::CDPTreasury::create_collateral_auctions(
				currency_id,
				collateral,
				target_stable_amount,
				who.clone(),
				true,
			)?;

			Ok(LiquidationStrategy::Auction)
		})()?;

		Self::deposit_event(Event::LiquidateUnsafeCDP(
			currency_id,
			who,
			collateral,
			bad_debt_value,
			liquidation_strategy,
		));
		Ok(())
	}


	// fn _offchain_worker() -> Result<(), OffchainErr> {
}

impl<T: Config> RiskManager<T::AccountId, CurrencyId, Balance, Balance> for Pallet<T> {
	fn get_bad_debt_value(currency_id: CurrencyId, debit_balance: Balance) -> Balance {
		Self::get_debit_value(currency_id, debit_balance)
	}

	fn check_position_valid(
		currency_id: CurrencyId,
		collateral_balance: Balance,
		debit_balance: Balance,
	) -> DispatchResult {
		if !debit_balance.is_zero() {
			let debit_value = Self::get_debit_value(currency_id, debit_balance);
			let feed_price = <T as Config>::PriceSource::get_relative_price(currency_id, T::GetStableCurrencyId::get())
				.ok_or(Error::<T>::InvalidFeedPrice)?;
			let collateral_ratio =
				Self::calculate_collateral_ratio(currency_id, collateral_balance, debit_balance, feed_price);

			// check the required collateral ratio
			if let Some(required_collateral_ratio) = Self::required_collateral_ratio(currency_id) {
				ensure!(
					collateral_ratio >= required_collateral_ratio,
					Error::<T>::BelowRequiredCollateralRatio
				);
			}

			// check the liquidation ratio
			ensure!(
				collateral_ratio >= Self::get_liquidation_ratio(currency_id),
				Error::<T>::BelowLiquidationRatio
			);

			// check the minimum_debit_value
			ensure!(
				debit_value >= T::MinimumDebitValue::get(),
				Error::<T>::RemainDebitValueTooSmall,
			);
		}

		Ok(())
	}

	fn check_debit_cap(currency_id: CurrencyId, total_debit_balance: Balance) -> DispatchResult {
		let hard_cap = Self::maximum_total_debit_value(currency_id);
		let total_debit_value = Self::get_debit_value(currency_id, total_debit_balance);

		ensure!(total_debit_value <= hard_cap, Error::<T>::ExceedDebitValueHardCap,);

		Ok(())
	}
}