#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::{pallet_prelude::*, transactional};
use frame_system::pallet_prelude::*;
use sp_std::{prelude::*, convert::TryInto, result::Result};
use sp_runtime::{
	traits::{BlakeTwo256,AccountIdConversion, Bounded, Convert, Hash, Saturating, StaticLookup, Zero},
	transaction_validity::{
		InvalidTransaction, TransactionPriority, TransactionSource, TransactionValidity, ValidTransaction,
	},
	DispatchError, DispatchResult, FixedPointNumber, ModuleId, RandomNumberGenerator, RuntimeDebug,
};

use orml_traits::{Change, Happened, MultiCurrency, MultiCurrencyExtended};
use primitives::{Rate, Ratio, ExchangeRate, Price, Amount, Balance, CurrencyId};
use primitives::risk::RiskManager;
use primitives::prices::PriceProvider;
use primitives::treasury::{CDPTreasury, CDPTreasuryExtended};

pub mod params;
pub use params::*;

pub use mypallet::*;
mod default_weight;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub trait WeightInfo {
	fn authorize() -> Weight;
	fn unauthorize() -> Weight;
	fn unauthorize_all(c: u32) -> Weight;
	fn adjust_loan() -> Weight;
	fn transfer_loan_from() -> Weight;

	fn set_collateral_params() -> Weight;
	fn set_global_params() -> Weight;
	fn liquidate_by_auction() -> Weight;
	fn liquidate_by_dex() -> Weight;
	fn settle() -> Weight;
}

// pub const OFFCHAIN_WORKER_DATA: &[u8] = b"bandot/cdp/data/";
// pub const OFFCHAIN_WORKER_LOCK: &[u8] = b"bandot/cdp/lock/";
// pub const OFFCHAIN_WORKER_MAX_ITERATIONS: &[u8] = b"bandot/cdp/max-iterations/";
// pub const LOCK_DURATION: u64 = 100;
// pub const DEFAULT_MAX_ITERATIONS: u32 = 1000;

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
	pub trait Config: frame_system::Config {
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		#[pallet::constant]
		type ModuleId: Get<ModuleId>;

		type Currency: MultiCurrencyExtended<
			Self::AccountId,
			CurrencyId = CurrencyId,
			Balance = Balance,
			Amount = Amount,
		>;

		type UpdateOrigin: EnsureOrigin<Self::Origin>;
		type PriceSource: PriceProvider<CurrencyId>;
		type CDPTreasury: CDPTreasuryExtended<Self::AccountId, Balance = Balance, CurrencyId = CurrencyId>;
		//type DEX: DEXManager<Self::AccountId, CurrencyId, Balance>;
		//type EmergencyShutdown: EmergencyShutdown;

		#[pallet::constant]
		type GetStableCurrencyId: Get<CurrencyId>; //bUSD
		#[pallet::constant]
		type CollateralCurrencyIds: Get<Vec<CurrencyId>>; // DOT

		#[pallet::constant]
		type DefaultLiquidationRatio: Get<Ratio>;	// LP ratio
		#[pallet::constant]
		type DefaultDebitExchangeRate: Get<ExchangeRate>; // exchange rate

		#[pallet::constant]
		type DefaultLiquidationPenalty: Get<Rate>; // penalty
		#[pallet::constant]
		type MinimumDebitValue: Get<Balance>; // min debit

		#[pallet::constant]
		type MaxSlippageSwapWithDEX: Get<Ratio>; // max slip

		// #[pallet::constant]
		// type UnsignedPriority: Get<TransactionPriority>;
		type OnUpdateLoan: Happened<(Self::AccountId, CurrencyId, Amount, Balance)>;
		type WeightInfo: WeightInfo;
	}

	#[pallet::error]
	pub enum Error<T> {
		NoAuthorization,
		AlreadyShutdown,

		DebitOverflow,
		DebitTooLow,
		CollateralOverflow,
		CollateralTooLow,
		AmountConvertFailed,

		ExceedDebitValueHardCap,
		BelowRequiredCollateralRatio,
		BelowLiquidationRatio,
		MustBeUnsafe,
		InvalidCollateralType,
		RemainDebitValueTooSmall,
		InvalidFeedPrice,
		NoDebitValue,
		MustAfterShutdown,
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(crate) fn deposit_event)]
	pub enum Event<T: Config> {
		Authorization(T::AccountId, T::AccountId, CurrencyId),
		UnAuthorization(T::AccountId, T::AccountId, CurrencyId),
		UnAuthorizationAll(T::AccountId),

		PositionUpdated(T::AccountId, CurrencyId, Amount, Amount),
		ConfiscateCollateralAndDebit(T::AccountId, CurrencyId, Balance, Balance),
		TransferLoan(T::AccountId, T::AccountId, CurrencyId),

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
	#[pallet::getter(fn authorization)]
	pub type Authorization<T: Config> =
	StorageDoubleMap<_, Twox64Concat, T::AccountId, Blake2_128Concat, (CurrencyId, T::AccountId), bool, ValueQuery>;


	#[pallet::storage]
	#[pallet::getter(fn positions)]
	pub type Positions<T: Config> = StorageDoubleMap<_, Twox64Concat, CurrencyId, Twox64Concat, T::AccountId, Position, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn total_positions)]
	pub type TotalPositions<T: Config> = StorageMap<_, Twox64Concat, CurrencyId, Position, ValueQuery>;

	/// exchange rate
	#[pallet::storage]
	#[pallet::getter(fn debit_exchange_rate)]
	pub type DebitExchangeRate<T: Config> = StorageMap<_, Twox64Concat, CurrencyId, ExchangeRate, OptionQuery>;

	/// stable fee
	#[pallet::storage]
	#[pallet::getter(fn global_stability_fee)]
	pub type GlobalStabilityFee<T: Config> = StorageValue<_, Rate, ValueQuery>;

	/// currency -> params
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
		// fn offchain_worker(now: T::BlockNumber) {
	}

	#[pallet::call]
	impl<T:Config> Pallet<T> {
		/// 修改仓位
		#[pallet::weight(<T as Config>::WeightInfo::adjust_loan())]
		#[transactional]
		pub fn adjust_loan(
			origin: OriginFor<T>,
			currency_id: CurrencyId,
			collateral_adjustment: Amount,
			debit_adjustment: Amount,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// not allowed to adjust the debit after system shutdown
			// if !debit_adjustment.is_zero() {
			// 	ensure!(!T::EmergencyShutdown::is_shutdown(), Error::<T>::AlreadyShutdown);
			// }
			Self::adjust_position(&who, currency_id, collateral_adjustment, debit_adjustment)?;
			Ok(().into())
		}

		#[pallet::weight(<T as Config>::WeightInfo::transfer_loan_from())]
		#[transactional]
		pub fn transfer_loan_from(
			origin: OriginFor<T>,
			currency_id: CurrencyId,
			from: <T::Lookup as StaticLookup>::Source,
		) -> DispatchResultWithPostInfo {
			let to = ensure_signed(origin)?;
			let from = T::Lookup::lookup(from)?;
			//ensure!(!T::EmergencyShutdown::is_shutdown(), Error::<T>::AlreadyShutdown);
			Self::check_authorization(&from, &to, currency_id)?;
			Self::transfer_loan(&from, &to, currency_id)?;
			Ok(().into())
		}
		/// 权限操作
		#[pallet::weight(<T as Config>::WeightInfo::authorize())]
		#[transactional]
		pub fn authorize(
			origin: OriginFor<T>,
			currency_id: CurrencyId,
			to: <T::Lookup as StaticLookup>::Source,
		) -> DispatchResultWithPostInfo {
			let from = ensure_signed(origin)?;
			let to = T::Lookup::lookup(to)?;
			<Authorization<T>>::insert(&from, (currency_id, &to), true);
			Self::deposit_event(Event::Authorization(from, to, currency_id));
			Ok(().into())
		}

		#[pallet::weight(<T as Config>::WeightInfo::unauthorize_all(<T as Config>::CollateralCurrencyIds::get().len() as u32))]
		#[transactional]
		pub fn unauthorize_all(origin: OriginFor<T>) -> DispatchResultWithPostInfo {
			let from = ensure_signed(origin)?;
			<Authorization<T>>::remove_prefix(&from);
			Self::deposit_event(Event::UnAuthorizationAll(from));
			Ok(().into())
		}

		#[pallet::weight(<T as Config>::WeightInfo::unauthorize())]
		#[transactional]
		pub fn unauthorize(
			origin: OriginFor<T>,
			currency_id: CurrencyId,
			to: <T::Lookup as StaticLookup>::Source,
		) -> DispatchResultWithPostInfo {
			let from = ensure_signed(origin)?;
			let to = T::Lookup::lookup(to)?;
			<Authorization<T>>::remove(&from, (currency_id, &to));
			Self::deposit_event(Event::UnAuthorization(from, to, currency_id));
			Ok(().into())
		}

		/// 清算 在shutdown offchain执行?
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

		/// 解决 在shutdown offchain执行?
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

		/// 设置global fee
		#[pallet::weight((T::WeightInfo::set_global_params(), DispatchClass::Operational))]
		#[transactional]
		pub fn set_global_params(origin: OriginFor<T>, global_stability_fee: Rate) -> DispatchResultWithPostInfo {
			T::UpdateOrigin::ensure_origin(origin)?;
			GlobalStabilityFee::<T>::put(global_stability_fee);
			Self::deposit_event(Event::GlobalStabilityFeeUpdated(global_stability_fee));
			Ok(().into())
		}

		/// 设置currencyId params
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
}

impl<T: Config> Pallet<T> {
	// fn submit_unsigned_liquidation_tx(currency_id: CurrencyId, who: T::AccountId) {
	// fn submit_unsigned_settlement_tx(currency_id: CurrencyId, who: T::AccountId) {
}
///params参数
impl<T: Config> Pallet<T> {
	/// 检查授权, 转移债务时候用
	fn check_authorization(from: &T::AccountId, to: &T::AccountId, currency_id: CurrencyId) -> DispatchResult {
		ensure!(
			from == to || Self::authorization(from, (currency_id, to)),
			Error::<T>::NoAuthorization
		);
		Ok(())
	}

	/// 现价抵押品率<params.清算率
	pub fn is_cdp_unsafe(currency_id: CurrencyId, collateral: Balance, debit: Balance) -> bool {
		let stable_currency_id = T::GetStableCurrencyId::get();
		// 从source获取
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
	// exchange_rate(currncy_id)*debit
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

	/// 解决cdp和debt
	pub fn settle_cdp_has_debit(who: T::AccountId, currency_id: CurrencyId) -> DispatchResult {
		let Position { collateral, debit } = Self::positions(currency_id, &who);
		ensure!(!debit.is_zero(), Error::<T>::NoDebitValue);

		// 没收所有 cdp to cdp treasury, 清零
		let settle_price: Price = T::PriceSource::get_relative_price(T::GetStableCurrencyId::get(), currency_id)
			.ok_or(Error::<T>::InvalidFeedPrice)?;
		let bad_debt_value = Self::get_debit_value(currency_id, debit);
		let confiscate_collateral_amount =
			sp_std::cmp::min(settle_price.saturating_mul_int(bad_debt_value), collateral);

		// 没收所有抵押和债务
		Self::confiscate_collateral_and_debit(&who, currency_id, confiscate_collateral_amount, debit)?;

		Self::deposit_event(Event::SettleCDPInDebit(currency_id, who));
		Ok(())
	}

	/// 清算不安全的CDP
	pub fn liquidate_unsafe_cdp(who: T::AccountId, currency_id: CurrencyId) -> DispatchResult {
		let Position { collateral, debit } = Self::positions(currency_id, &who);

		// 判断是否不安全
		ensure!(
			Self::is_cdp_unsafe(currency_id, collateral, debit),
			Error::<T>::MustBeUnsafe
		);

		// 没收
		Self::confiscate_collateral_and_debit(&who, currency_id, collateral, debit)?;

		let bad_debt_value = Self::get_debit_value(currency_id, debit);
		let target_stable_amount = Self::get_liquidation_penalty(currency_id).saturating_mul_acc_int(bad_debt_value);

		// 使用dex先进行卖出,不够就出现拍卖
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

			// 启动抵押品拍卖
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

	// TODO:把loan合并
	#[transactional]
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

		Self::update_loan(who, currency_id, collateral_adjustment, debit_adjustment)?;

		let collateral_balance_adjustment = Self::balance_try_from_amount_abs(collateral_adjustment)?;
		let debit_balance_adjustment = Self::balance_try_from_amount_abs(debit_adjustment)?;
		let module_account = Self::account_id();
		// 抵押品 人呢<->loan
		if collateral_adjustment.is_positive() {
			T::Currency::transfer(currency_id, who, &module_account, collateral_balance_adjustment)?;
		} else if collateral_adjustment.is_negative() {
			T::Currency::transfer(currency_id, &module_account, who, collateral_balance_adjustment)?;
		}
		// debit过程, treasy发或者烧
		if debit_adjustment.is_positive() {
			// 从riskmanager 查看debit上限
			<Self as RiskManager<T::AccountId>>::check_debit_cap(currency_id, Self::total_positions(currency_id).debit)?;

			// 发debit
			T::CDPTreasury::issue_debit(who, crate::DebitExchangeRateConvertor::<T>::convert((currency_id, debit_balance_adjustment)), true)?;
		} else if debit_adjustment.is_negative() {
			T::CDPTreasury::burn_debit(who, crate::DebitExchangeRateConvertor::<T>::convert((currency_id, debit_balance_adjustment)))?;
		}

		// 检查position
		let Position { collateral, debit } = Self::positions(currency_id, who);
		<Self as RiskManager<T::AccountId>>::check_position_valid(currency_id, collateral, debit)?;

		Self::deposit_event(Event::PositionUpdated(
			who.clone(),
			currency_id,
			collateral_adjustment,
			debit_adjustment,
		));
		Ok(())
	}

	#[transactional]
	pub fn confiscate_collateral_and_debit(
		who: &T::AccountId,
		currency_id: CurrencyId,
		collateral_confiscate: Balance,
		debit_decrease: Balance,
	) -> DispatchResult {
		// convert balance type to amount type
		let collateral_adjustment = Self::amount_try_from_balance(collateral_confiscate)?;
		let debit_adjustment = Self::amount_try_from_balance(debit_decrease)?;

		// 从loan->treasury
		T::CDPTreasury::deposit_collateral(&Self::account_id(), currency_id, collateral_confiscate)?;

		// treasury增发坏账
		let bad_debt_value = <Self as RiskManager<T::AccountId>>::get_bad_debt_value(currency_id, debit_decrease);
		T::CDPTreasury::on_system_debit(bad_debt_value)?;

		// 减少who的抵押品和debit
		Self::update_loan(
			&who,
			currency_id,
			collateral_adjustment.saturating_neg(),
			debit_adjustment.saturating_neg(),
		)?;

		Self::deposit_event(Event::ConfiscateCollateralAndDebit(
			who.clone(),
			currency_id,
			collateral_confiscate,
			debit_decrease,
		));
		Ok(())
	}

	/// 转移loan
	pub fn transfer_loan(from: &T::AccountId, to: &T::AccountId, currency_id: CurrencyId) -> DispatchResult {
		// get `from` position data
		let Position { collateral, debit } = Self::positions(currency_id, from);

		let Position {
			collateral: to_collateral,
			debit: to_debit,
		} = Self::positions(currency_id, to);

		let new_to_collateral_balance = to_collateral
			.checked_add(collateral)
			.expect("existing collateral balance cannot overflow; qed");
		let new_to_debit_balance = to_debit
			.checked_add(debit)
			.expect("existing debit balance cannot overflow; qed");

		// 查看仓位满足
		<Self as RiskManager<T::AccountId>>::check_position_valid(currency_id, new_to_collateral_balance, new_to_debit_balance)?;

		// balance -> amount
		let collateral_adjustment = Self::amount_try_from_balance(collateral)?;
		let debit_adjustment = Self::amount_try_from_balance(debit)?;

		// 减少原来的 增加to的
		Self::update_loan(
			from,
			currency_id,
			collateral_adjustment.saturating_neg(),
			debit_adjustment.saturating_neg(),
		)?;
		Self::update_loan(to, currency_id, collateral_adjustment, debit_adjustment)?;

		Self::deposit_event(Event::TransferLoan(from.clone(), to.clone(), currency_id));
		Ok(())
	}

	fn update_loan(
		who: &T::AccountId,
		currency_id: CurrencyId,
		collateral_adjustment: Amount,
		debit_adjustment: Amount,
	) -> DispatchResult {
		let collateral_balance = Self::balance_try_from_amount_abs(collateral_adjustment)?;
		let debit_balance = Self::balance_try_from_amount_abs(debit_adjustment)?;
		// 修改who, currency_id仓位
		<Positions<T>>::try_mutate_exists(currency_id, who, |may_be_position| -> DispatchResult {
			let mut p = may_be_position.take().unwrap_or_default();
			let new_collateral = if collateral_adjustment.is_positive() {
				p.collateral
					.checked_add(collateral_balance)
					.ok_or(Error::<T>::CollateralOverflow)
			} else {
				p.collateral
					.checked_sub(collateral_balance)
					.ok_or(Error::<T>::CollateralTooLow)
			}?;
			let new_debit = if debit_adjustment.is_positive() {
				p.debit.checked_add(debit_balance).ok_or(Error::<T>::DebitOverflow)
			} else {
				p.debit.checked_sub(debit_balance).ok_or(Error::<T>::DebitTooLow)
			}?;

			// increase account ref if new position
			if p.collateral.is_zero() && p.debit.is_zero() {
				if frame_system::Module::<T>::inc_consumers(who).is_err() {
					// No providers for the locks. This is impossible under normal circumstances
					// since the funds that are under the lock will themselves be stored in the
					// account and therefore will need a reference.
					frame_support::debug::warn!(
						"Warning: Attempt to introduce lock consumer reference, yet no providers. \
						This is unexpected but should be safe."
					);
				}
			}

			p.collateral = new_collateral;

			// 触发回调
			T::OnUpdateLoan::happened(&(who.clone(), currency_id, debit_adjustment, p.debit));
			p.debit = new_debit;

			if p.collateral.is_zero() && p.debit.is_zero() {
				// decrease account ref if zero position
				frame_system::Module::<T>::dec_consumers(who);

				// remove position storage if zero position
				*may_be_position = None;
			} else {
				*may_be_position = Some(p);
			}

			Ok(())
		})?;
		// 修改TotalPositions
		TotalPositions::<T>::try_mutate(currency_id, |total_positions| -> DispatchResult {
			total_positions.collateral = if collateral_adjustment.is_positive() {
				total_positions
					.collateral
					.checked_add(collateral_balance)
					.ok_or(Error::<T>::CollateralOverflow)
			} else {
				total_positions
					.collateral
					.checked_sub(collateral_balance)
					.ok_or(Error::<T>::CollateralTooLow)
			}?;

			total_positions.debit = if debit_adjustment.is_positive() {
				total_positions
					.debit
					.checked_add(debit_balance)
					.ok_or(Error::<T>::DebitOverflow)
			} else {
				total_positions
					.debit
					.checked_sub(debit_balance)
					.ok_or(Error::<T>::DebitTooLow)
			}?;

			Ok(())
		})
	}

	// fn _offchain_worker() -> Result<(), OffchainErr> {
}

impl<T: Config> Pallet<T> {
	pub fn account_id() -> T::AccountId {
		T::ModuleId::get().into_account()
	}

	/// Convert `Balance` to `Amount`.
	fn amount_try_from_balance(b: Balance) -> Result<Amount, Error<T>> {
		TryInto::<Amount>::try_into(b).map_err(|_| Error::<T>::AmountConvertFailed)
	}

	/// Convert the absolute value of `Amount` to `Balance`.
	fn balance_try_from_amount_abs(a: Amount) -> Result<Balance, Error<T>> {
		TryInto::<Balance>::try_into(a.saturating_abs()).map_err(|_| Error::<T>::AmountConvertFailed)
	}
}

impl<T: Config> RiskManager<T::AccountId> for Pallet<T> {
	type CurrencyId = CurrencyId;
	type Balance = Balance;
	type DebitBalance = Balance;

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

			// 抵押物/debit >= required_collateral_ratio 和 get_liquidation_ratio
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