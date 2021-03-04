#![cfg_attr(not(feature = "std"), no_std)]
// 可与cdp合并
use frame_support::{pallet_prelude::*, transactional};
use orml_traits::{Happened, MultiCurrency, MultiCurrencyExtended};
use sp_runtime::{
	traits::{AccountIdConversion, Convert, Zero},
	DispatchResult, ModuleId, RuntimeDebug,
};
use sp_std::{convert::TryInto, result};
use primitives::{Amount, Balance, CurrencyId};
use primitives::treasury::CDPTreasury;
use primitives::risk::RiskManager;

pub use mypallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[derive(Encode, Decode, Eq, PartialEq, Copy, Clone, RuntimeDebug, Default)]
pub struct Position {
	/// The amount of collateral.
	pub collateral: Balance,
	/// The amount of debit.
	pub debit: Balance,
}

#[frame_support::pallet]
pub mod mypallet {
	use super::*;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		type Currency: MultiCurrencyExtended<
			Self::AccountId,
			CurrencyId = CurrencyId,
			Balance = Balance,
			Amount = Amount,
		>;

		type CDPTreasury: CDPTreasury<Self::AccountId, Balance = Balance, CurrencyId = CurrencyId>;
		type RiskManager: RiskManager<Self::AccountId, CurrencyId, Balance, Balance>;

		#[pallet::constant]
		type ModuleId: Get<ModuleId>;

		type Convert: Convert<(CurrencyId, Balance), Balance>;
		type OnUpdateLoan: Happened<(Self::AccountId, CurrencyId, Amount, Balance)>;
	}

	#[pallet::error]
	pub enum Error<T> {
		DebitOverflow,
		DebitTooLow,
		CollateralOverflow,
		CollateralTooLow,
		AmountConvertFailed,
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(crate) fn deposit_event)]
	pub enum Event<T: Config> {
		PositionUpdated(T::AccountId, CurrencyId, Amount, Amount),
		ConfiscateCollateralAndDebit(T::AccountId, CurrencyId, Balance, Balance),
		TransferLoan(T::AccountId, T::AccountId, CurrencyId),
	}

	#[pallet::storage]
	#[pallet::getter(fn positions)]
	pub type Positions<T: Config> =
	StorageDoubleMap<_, Twox64Concat, CurrencyId, Twox64Concat, T::AccountId, Position, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn total_positions)]
	pub type TotalPositions<T: Config> = StorageMap<_, Twox64Concat, CurrencyId, Position, ValueQuery>;

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::hooks]
	impl<T: Config> Hooks<T::BlockNumber> for Pallet<T> {}

	#[pallet::call]
	impl<T:Config> Pallet<T> {}
}

impl<T: Config> Pallet<T> {
	pub fn account_id() -> T::AccountId {
		T::ModuleId::get().into_account()
	}

	// 没收
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

		// transfer collateral to cdp treasury
		T::CDPTreasury::deposit_collateral(&Self::account_id(), currency_id, collateral_confiscate)?;

		// deposit debit to cdp treasury
		let bad_debt_value = T::RiskManager::get_bad_debt_value(currency_id, debit_decrease);
		T::CDPTreasury::on_system_debit(bad_debt_value)?;

		// update loan
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

	// 调整
	#[transactional]
	pub fn adjust_position(
		who: &T::AccountId,
		currency_id: CurrencyId,
		collateral_adjustment: Amount,
		debit_adjustment: Amount,
	) -> DispatchResult {
		// mutate collateral and debit
		Self::update_loan(who, currency_id, collateral_adjustment, debit_adjustment)?;

		let collateral_balance_adjustment = Self::balance_try_from_amount_abs(collateral_adjustment)?;
		let debit_balance_adjustment = Self::balance_try_from_amount_abs(debit_adjustment)?;
		let module_account = Self::account_id();

		if collateral_adjustment.is_positive() {
			T::Currency::transfer(currency_id, who, &module_account, collateral_balance_adjustment)?;
		} else if collateral_adjustment.is_negative() {
			T::Currency::transfer(currency_id, &module_account, who, collateral_balance_adjustment)?;
		}

		if debit_adjustment.is_positive() {
			// check debit cap when increase debit
			T::RiskManager::check_debit_cap(currency_id, Self::total_positions(currency_id).debit)?;

			// issue debit with collateral backed by cdp treasury
			T::CDPTreasury::issue_debit(who, T::Convert::convert((currency_id, debit_balance_adjustment)), true)?;
		} else if debit_adjustment.is_negative() {
			// repay debit
			// burn debit by cdp treasury
			T::CDPTreasury::burn_debit(who, T::Convert::convert((currency_id, debit_balance_adjustment)))?;
		}

		// ensure pass risk check
		let Position { collateral, debit } = Self::positions(currency_id, who);
		T::RiskManager::check_position_valid(currency_id, collateral, debit)?;

		Self::deposit_event(Event::PositionUpdated(
			who.clone(),
			currency_id,
			collateral_adjustment,
			debit_adjustment,
		));
		Ok(())
	}

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

		// check new position
		T::RiskManager::check_position_valid(currency_id, new_to_collateral_balance, new_to_debit_balance)?;

		// balance -> amount
		let collateral_adjustment = Self::amount_try_from_balance(collateral)?;
		let debit_adjustment = Self::amount_try_from_balance(debit)?;

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
}

impl<T: Config> Pallet<T> {
	/// Convert `Balance` to `Amount`.
	fn amount_try_from_balance(b: Balance) -> result::Result<Amount, Error<T>> {
		TryInto::<Amount>::try_into(b).map_err(|_| Error::<T>::AmountConvertFailed)
	}

	/// Convert the absolute value of `Amount` to `Balance`.
	fn balance_try_from_amount_abs(a: Amount) -> result::Result<Balance, Error<T>> {
		TryInto::<Balance>::try_into(a.saturating_abs()).map_err(|_| Error::<T>::AmountConvertFailed)
	}
}
