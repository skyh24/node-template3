#![cfg_attr(not(feature = "std"), no_std)]

use frame_support::{pallet_prelude::*, transactional};
use frame_system::pallet_prelude::*;
use sp_runtime::{
	DispatchResult, FixedPointNumber,
	traits::{BlakeTwo256, CheckedDiv, Hash, Saturating, Zero},
};

use orml_traits::{Auction, AuctionHandler, Change, MultiCurrency, OnNewBidResult};
use orml_utilities::{IterableStorageMapExtended, OffchainErr};

use primitives::{AuctionId, Balance, Rate, CurrencyId};
use primitives::prices::PriceProvider;
use primitives::auction::AuctionManager;
use primitives::treasury::{CDPTreasury, CDPTreasuryExtended};

pub mod auction_items;
pub use crate::auction_items::*;

pub use mypallet::*;
mod default_weight;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub trait WeightInfo {
	fn cancel_collateral_auction() -> Weight;
}

pub const OFFCHAIN_WORKER_DATA: &[u8] = b"bandot/auction/data/";
pub const OFFCHAIN_WORKER_LOCK: &[u8] = b"bandot/auction/lock/";
pub const OFFCHAIN_WORKER_MAX_ITERATIONS: &[u8] = b"bandot/auction/max-iterations/";
pub const LOCK_DURATION: u64 = 100;
pub const DEFAULT_MAX_ITERATIONS: u32 = 1000;

#[frame_support::pallet]
pub mod mypallet {
	use super::*;

	#[pallet::config]
	pub trait Config: frame_system::Config { // + SendTransactionTypes<Call<Self>> {
		type Event: From<Event<Self>> + IsType<<Self as frame_system::Config>::Event>;

		type Currency: MultiCurrency<Self::AccountId, CurrencyId = CurrencyId, Balance = Balance>;
		type Auction: Auction<Self::AccountId, Self::BlockNumber, AuctionId = AuctionId, Balance = Balance>;
		type CDPTreasury: CDPTreasuryExtended<Self::AccountId, Balance = Balance, CurrencyId = CurrencyId>;
		type PriceSource: PriceProvider<CurrencyId>;
		// type DEX: DEXManager<Self::AccountId, CurrencyId, Balance>;
		// type EmergencyShutdown: EmergencyShutdown;

		#[pallet::constant]
		type AuctionTimeToClose: Get<Self::BlockNumber>;
		#[pallet::constant]
		type MinimumIncrementSize: Get<Rate>;
		#[pallet::constant]
		type AuctionDurationSoftCap: Get<Self::BlockNumber>;

		#[pallet::constant]
		type GetStableCurrencyId: Get<CurrencyId>;
		#[pallet::constant]
		type GetNativeCurrencyId: Get<CurrencyId>;

		// #[pallet::constant]
		// type UnsignedPriority: Get<TransactionPriority>;
		type WeightInfo: WeightInfo;

	}

	#[pallet::error]
	pub enum Error<T> {
		AuctionNotExists,
		InReverseStage,
		InvalidFeedPrice,
		MustAfterShutdown,
		InvalidBidPrice,
		InvalidAmount,
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(crate) fn deposit_event)]
	pub enum Event<T: Config> {
		NewCollateralAuction(AuctionId, CurrencyId, Balance, Balance),
		NewDebitAuction(AuctionId, Balance, Balance),
		NewSurplusAuction(AuctionId, Balance),
		CancelAuction(AuctionId),
		CollateralAuctionDealt(AuctionId, CurrencyId, Balance, T::AccountId, Balance),
		SurplusAuctionDealt(AuctionId, Balance, T::AccountId, Balance),
		DebitAuctionDealt(AuctionId, Balance, T::AccountId, Balance),
		DEXTakeCollateralAuction(AuctionId, CurrencyId, Balance, Balance),
	}

	/// 抵押物
	#[pallet::storage]
	#[pallet::getter(fn collateral_auctions)]
	pub type CollateralAuctions<T: Config> =
	StorageMap<_, Twox64Concat, AuctionId, CollateralAuctionItem<T::AccountId, T::BlockNumber>, OptionQuery>;

	/// 债务
	#[pallet::storage]
	#[pallet::getter(fn debit_auctions)]
	pub type DebitAuctions<T: Config> =
	StorageMap<_, Twox64Concat, AuctionId, DebitAuctionItem<T::BlockNumber>, OptionQuery>;

	/// 剩余
	#[pallet::storage]
	#[pallet::getter(fn surplus_auctions)]
	pub type SurplusAuctions<T: Config> =
	StorageMap<_, Twox64Concat, AuctionId, SurplusAuctionItem<T::BlockNumber>, OptionQuery>;

	/// 计算数量
	#[pallet::storage]
	#[pallet::getter(fn total_target_in_auction)]
	pub type TotalTargetInAuction<T: Config> = StorageValue<_, Balance, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn total_collateral_in_auction)]
	pub type TotalCollateralInAuction<T: Config> = StorageMap<_, Twox64Concat, CurrencyId, Balance, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn total_debit_in_auction)]
	pub type TotalDebitInAuction<T: Config> = StorageValue<_, Balance, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn total_surplus_in_auction)]
	pub type TotalSurplusInAuction<T: Config> = StorageValue<_, Balance, ValueQuery>;

	#[pallet::pallet]
	#[pallet::generate_store(pub(super) trait Store)]
	pub struct Pallet<T>(_);

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		// fn offchain_worker(now: T::BlockNumber) {

	}

	// #[pallet::validate_unsigned]
	// impl<T: Config> ValidateUnsigned for Pallet<T> {

	#[pallet::call]
	impl<T:Config> Pallet<T> {
		#[pallet::weight(T::WeightInfo::cancel_collateral_auction())]
		#[transactional]
		pub fn cancel(origin: OriginFor<T>, id: AuctionId) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;
			//ensure!(T::EmergencyShutdown
			<Self as AuctionManager<T::AccountId>>::cancel_auction(id)?;
			Self::deposit_event(Event::CancelAuction(id));
			Ok(().into())
		}

	}
}

/// offchain
impl<T: Config> Pallet<T> {
	// fn submit_cancel_auction_tx(auction_id: AuctionId) {
	// fn _offchain_worker() -> Result<(), OffchainErr> {
}

/// cacel handler
impl<T: Config> Pallet<T> {
	/// 最后一个bid
	fn get_last_bid(auction_id: AuctionId) -> Option<(T::AccountId, Balance)> {
		T::Auction::auction_info(auction_id).and_then(|auction_info| auction_info.bid)
	}

	/// 取消surplus
	fn cancel_surplus_auction(id: AuctionId, surplus_auction: SurplusAuctionItem<T::BlockNumber>) -> DispatchResult {
		if let Some((bidder, bid_price)) = Self::get_last_bid(id) {
			// deposit BDT last bid_price
			T::Currency::deposit(T::GetNativeCurrencyId::get(), &bidder, bid_price)?;
			// 消除bidder引用
			frame_system::Module::<T>::dec_consumers(&bidder);
		}
		// 修改 TotalSurplusInAuction
		TotalSurplusInAuction::<T>::mutate(|balance| *balance = balance.saturating_sub(surplus_auction.amount));
		Ok(())
	}
	/// 取消debit
	fn cancel_debit_auction(id: AuctionId, debit_auction: DebitAuctionItem<T::BlockNumber>) -> DispatchResult {
		if let Some((bidder, _)) = Self::get_last_bid(id) {
			// issue_debit fix
			T::CDPTreasury::issue_debit(&bidder, debit_auction.fix, false)?;
			// 消除bidder引用
			frame_system::Module::<T>::dec_consumers(&bidder);
		}
		// 修改 TotalDebitInAuction
		TotalDebitInAuction::<T>::mutate(|balance| *balance = balance.saturating_sub(debit_auction.fix));
		Ok(())
	}
	/// 取消collateral
	fn cancel_collateral_auction(id: AuctionId, collateral_auction: CollateralAuctionItem<T::AccountId, T::BlockNumber>) -> DispatchResult {
		let last_bid = Self::get_last_bid(id);

		// 不可以小过之前的拍卖价
		if let Some((_, bid_price)) = last_bid {
			ensure!(
				!collateral_auction.in_reverse_stage(bid_price),
				Error::<T>::InReverseStage,
			);
		}

		// 计算剩余的抵押品
		let stable_currency_id = T::GetStableCurrencyId::get();
		let settle_price = T::PriceSource::get_relative_price(stable_currency_id, collateral_auction.currency_id)
			.ok_or(Error::<T>::InvalidFeedPrice)?; // settle = 1 / currency_price

		let confiscate_collateral_amount = if collateral_auction.always_forward() {
			collateral_auction.amount
		} else {
			sp_std::cmp::min(
				settle_price.saturating_mul_int(collateral_auction.target), // target / price = amount
				collateral_auction.amount,
			)
		};
		let refund_collateral_amount = collateral_auction.amount.saturating_sub(confiscate_collateral_amount);

		// 归还剩余
		T::CDPTreasury::withdraw_collateral(
			&collateral_auction.refund_recipient,
			collateral_auction.currency_id,
			refund_collateral_amount,
		)?;

		// 由于区块链是异步的,检查仍有无则返还.方法2也可以在cancel之前锁住
		if let Some((bidder, bid_price)) = last_bid {
			T::CDPTreasury::issue_debit(&bidder, bid_price, false)?;
			// 消除bidder引用
			frame_system::Module::<T>::dec_consumers(&bidder);
		}

		// 消除抵押人引用
		frame_system::Module::<T>::dec_consumers(&collateral_auction.refund_recipient);

		// 取消currency和总拍卖
		TotalCollateralInAuction::<T>::mutate(collateral_auction.currency_id, |balance| {
			*balance = balance.saturating_sub(collateral_auction.amount)
		});
		TotalTargetInAuction::<T>::mutate(|balance| *balance = balance.saturating_sub(collateral_auction.target));

		Ok(())
	}
}

/// bid handler
impl<T: Config> Pallet<T> {
	/// mi * target >= new - last
	fn check_minimum_increment(
		new_price: Balance,
		last_price: Balance,
		target_price: Balance,
		minimum_increment: Rate,
	) -> bool {
		if let (Some(target), Some(result)) = (
			minimum_increment.checked_mul_int(sp_std::cmp::max(target_price, last_price)),
			new_price.checked_sub(last_price),
		) {
			result >= target
		} else {
			false
		}
	}

	/// 到达AuctionDurationSoftCap, 加倍
	fn get_minimum_increment_size(now: T::BlockNumber, start_block: T::BlockNumber) -> Rate {
		if now >= start_block + T::AuctionDurationSoftCap::get() {
			T::MinimumIncrementSize::get().saturating_mul(Rate::saturating_from_integer(2))
		} else {
			T::MinimumIncrementSize::get()
		}
	}

	/// 到达AuctionDurationSoftCap, 减半
	fn get_auction_time_to_close(now: T::BlockNumber, start_block: T::BlockNumber) -> T::BlockNumber {
		if now >= start_block + T::AuctionDurationSoftCap::get() {
			T::AuctionTimeToClose::get()
				.checked_div(&2u32.into())
				.expect("cannot overflow with positive divisor; qed")
		} else {
			T::AuctionTimeToClose::get()
		}
	}

	/// 转换拍卖人,就是在system中引用
	fn swap_bidders(new_bidder: &T::AccountId, last_bidder: Option<&T::AccountId>) {
		if frame_system::Module::<T>::inc_consumers(new_bidder).is_err() {
			frame_support::debug::warn!(
				"Warning: Attempt to introduce lock consumer reference, yet no providers. \
				This is unexpected but should be safe."
			);
		}

		if let Some(who) = last_bidder {
			frame_system::Module::<T>::dec_consumers(who);
		}
	}

	/// 拍卖抵押品时候
	#[transactional]
	pub fn collateral_auction_bid_handler(
		now: T::BlockNumber,
		id: AuctionId,
		new_bid: (T::AccountId, Balance),
		last_bid: Option<(T::AccountId, Balance)>,
	) -> sp_std::result::Result<T::BlockNumber, DispatchError> {
		let (new_bidder, new_bid_price) = new_bid;
		ensure!(!new_bid_price.is_zero(), Error::<T>::InvalidBidPrice);

		<CollateralAuctions<T>>::try_mutate_exists(
			id,
			|collateral_auction| -> sp_std::result::Result<T::BlockNumber, DispatchError> {
				let mut collateral_auction = collateral_auction.as_mut().ok_or(Error::<T>::AuctionNotExists)?;
				let last_bid_price = last_bid.clone().map_or(Zero::zero(), |(_, price)| price); // get last bid price

				// 判断大于最低增长值
				ensure!(
					Self::check_minimum_increment(
						new_bid_price,
						last_bid_price,
						collateral_auction.target,
						Self::get_minimum_increment_size(now, collateral_auction.start_time), // 最小size
					),
					Error::<T>::InvalidBidPrice
				);

				let last_bidder = last_bid.as_ref().map(|(who, _)| who);
				let mut payment = collateral_auction.payment_amount(new_bid_price);

				// 归还stable给旧bidder
				if let Some(last_bidder) = last_bidder {
					let refund = collateral_auction.payment_amount(last_bid_price);
					T::Currency::transfer(T::GetStableCurrencyId::get(), &new_bidder, last_bidder, refund)?;

					payment = payment
						.checked_sub(refund)
						.ok_or(Error::<T>::InvalidBidPrice)?;
				}

				// 剩余转到cdp
				T::CDPTreasury::deposit_surplus(&new_bidder, payment)?;

				// 大于target, 则归还抵押品
				if collateral_auction.in_reverse_stage(new_bid_price) {
					let new_collateral_amount = collateral_auction.collateral_amount(last_bid_price, new_bid_price);
					let refund_collateral_amount = collateral_auction.amount.saturating_sub(new_collateral_amount);

					if !refund_collateral_amount.is_zero() {
						T::CDPTreasury::withdraw_collateral(
							&(collateral_auction.refund_recipient),
							collateral_auction.currency_id,
							refund_collateral_amount,
						)?;

						// update total collateral in auction after refund
						TotalCollateralInAuction::<T>::mutate(collateral_auction.currency_id, |balance| {
							*balance = balance.saturating_sub(refund_collateral_amount)
						});
						collateral_auction.amount = new_collateral_amount;
					}
				}

				Self::swap_bidders(&new_bidder, last_bidder);
				Ok(now + Self::get_auction_time_to_close(now, collateral_auction.start_time))
			},
		)
	}

	/// 拍卖债务时候
	#[transactional]
	pub fn debit_auction_bid_handler(
		now: T::BlockNumber,
		id: AuctionId,
		new_bid: (T::AccountId, Balance),
		last_bid: Option<(T::AccountId, Balance)>,
	) -> sp_std::result::Result<T::BlockNumber, DispatchError> {
		<DebitAuctions<T>>::try_mutate_exists(
			id,
			|debit_auction| -> sp_std::result::Result<T::BlockNumber, DispatchError> {
				let mut debit_auction = debit_auction.as_mut().ok_or(Error::<T>::AuctionNotExists)?;
				let (new_bidder, new_bid_price) = new_bid;
				let last_bid_price = last_bid.clone().map_or(Zero::zero(), |(_, price)| price); // get last bid price

				// 判断大于最低增长值
				ensure!(
					Self::check_minimum_increment(
						new_bid_price,
						last_bid_price,
						debit_auction.fix,
						Self::get_minimum_increment_size(now, debit_auction.start_time),
					) && new_bid_price >= debit_auction.fix,
					Error::<T>::InvalidBidPrice,
				);

				let last_bidder = last_bid.as_ref().map(|(who, _)| who);

				if let Some(last_bidder) = last_bidder {
					// 归还stable给旧bidder
					T::Currency::transfer(
						T::GetStableCurrencyId::get(),
						&new_bidder,
						last_bidder,
						debit_auction.fix,
					)?;
				} else {
					// 前面没有bidder,则转到cdp
					T::CDPTreasury::deposit_surplus(&new_bidder, debit_auction.fix)?;
				}

				Self::swap_bidders(&new_bidder, last_bidder);

				debit_auction.amount = debit_auction.amount_for_sale(last_bid_price, new_bid_price);
				Ok(now + Self::get_auction_time_to_close(now, debit_auction.start_time))
			},
		)
	}

	#[transactional]
	pub fn surplus_auction_bid_handler(
		now: T::BlockNumber,
		id: AuctionId,
		new_bid: (T::AccountId, Balance),
		last_bid: Option<(T::AccountId, Balance)>,
	) -> sp_std::result::Result<T::BlockNumber, DispatchError> {
		let (new_bidder, new_bid_price) = new_bid;
		ensure!(!new_bid_price.is_zero(), Error::<T>::InvalidBidPrice);

		let surplus_auction = Self::surplus_auctions(id).ok_or(Error::<T>::AuctionNotExists)?;
		let last_bid_price = last_bid.clone().map_or(Zero::zero(), |(_, price)| price); // get last bid price
		let native_currency_id = T::GetNativeCurrencyId::get();

		ensure!(
			Self::check_minimum_increment(
				new_bid_price,
				last_bid_price,
				Zero::zero(),
				Self::get_minimum_increment_size(now, surplus_auction.start_time),
			),
			Error::<T>::InvalidBidPrice,
		);

		let last_bidder = last_bid.as_ref().map(|(who, _)| who);

		let burn_amount = if let Some(last_bidder) = last_bidder {
			// refund last bidder
			T::Currency::transfer(native_currency_id, &new_bidder, last_bidder, last_bid_price)?;
			new_bid_price.saturating_sub(last_bid_price)
		} else {
			new_bid_price
		};

		// 拍卖销毁原生代币
		T::Currency::withdraw(native_currency_id, &new_bidder, burn_amount)?;

		Self::swap_bidders(&new_bidder, last_bidder);

		Ok(now + Self::get_auction_time_to_close(now, surplus_auction.start_time))
	}
}

/// end handler
impl<T: Config> Pallet<T> {
	fn collateral_auction_end_handler(
		auction_id: AuctionId,
		collateral_auction: CollateralAuctionItem<T::AccountId, T::BlockNumber>,
		winner: Option<(T::AccountId, Balance)>,
	) {
		if let Some((bidder, bid_price)) = winner {
			let mut should_deal = true;

			// 如果没达到targe就会直接放入dex
			if !collateral_auction.in_reverse_stage(bid_price)
			// 	&& bid_price
			// 	< T::DEX::get_swap_target_amount(
			// 	&[collateral_auction.currency_id, T::GetStableCurrencyId::get()],
			// 	collateral_auction.amount,
			// 	None,
			// ).unwrap_or_default()
			{
				// 在teasury中swap
				if let Ok(stable_amount) = T::CDPTreasury::swap_exact_collateral_in_auction_to_stable(
					collateral_auction.currency_id,
					collateral_auction.amount,
					Zero::zero(),
					None,
				) {
					// swap successfully, will not deal
					should_deal = false;

					// 向bidder发bidprice
					let _ = T::CDPTreasury::issue_debit(&bidder, bid_price, false);

					if collateral_auction.in_reverse_stage(stable_amount) {
						// 从swap结果-target返回抵押人
						let refund_amount = stable_amount
							.checked_sub(collateral_auction.target)
							.expect("ensured stable_amount > target; qed");
						// it shouldn't fail and affect the process.
						// but even it failed, just the winner did not get the refund amount. it can be
						// fixed by treasury council.
						let _ = T::CDPTreasury::issue_debit(&collateral_auction.refund_recipient, refund_amount, false);
					}

					Self::deposit_event(Event::DEXTakeCollateralAuction(
						auction_id,
						collateral_auction.currency_id,
						collateral_auction.amount,
						stable_amount,
					));
				}
			}

			if should_deal {
				// 转移抵押品给bidder
				let _ = T::CDPTreasury::withdraw_collateral(
					&bidder,
					collateral_auction.currency_id,
					collateral_auction.amount,
				);

				let payment_amount = collateral_auction.payment_amount(bid_price);
				Self::deposit_event(Event::CollateralAuctionDealt(
					auction_id,
					collateral_auction.currency_id,
					collateral_auction.amount,
					bidder,
					payment_amount,
				));
			}
		} else {
			Self::deposit_event(Event::CancelAuction(auction_id));
		}

		// decrement recipient account reference
		frame_system::Module::<T>::dec_consumers(&collateral_auction.refund_recipient);

		// update auction records
		TotalCollateralInAuction::<T>::mutate(collateral_auction.currency_id, |balance| {
			*balance = balance.saturating_sub(collateral_auction.amount)
		});
		TotalTargetInAuction::<T>::mutate(|balance| *balance = balance.saturating_sub(collateral_auction.target));
	}

	fn debit_auction_end_handler(
		auction_id: AuctionId,
		debit_auction: DebitAuctionItem<T::BlockNumber>,
		winner: Option<(T::AccountId, Balance)>,
	) {
		if let Some((bidder, _)) = winner {
			// 转移BDT到cdp
			let _ = T::Currency::deposit(T::GetNativeCurrencyId::get(), &bidder, debit_auction.amount);

			Self::deposit_event(Event::DebitAuctionDealt(
				auction_id,
				debit_auction.amount,
				bidder,
				debit_auction.fix,
			));
		} else {
			Self::deposit_event(Event::CancelAuction(auction_id));
		}

		TotalDebitInAuction::<T>::mutate(|balance| *balance = balance.saturating_sub(debit_auction.fix));
	}

	fn surplus_auction_end_handler(
		auction_id: AuctionId,
		surplus_auction: SurplusAuctionItem<T::BlockNumber>,
		winner: Option<(T::AccountId, Balance)>,
	) {
		if let Some((bidder, bid_price)) = winner {
			// deposit unbacked stable token to winner by CDP treasury, it shouldn't fail
			// and affect the process. but even it failed, just the winner did not get the
			// amount. it can be fixed by treasury council.
			let _ = T::CDPTreasury::issue_debit(&bidder, surplus_auction.amount, false);

			Self::deposit_event(Event::SurplusAuctionDealt(
				auction_id,
				surplus_auction.amount,
				bidder,
				bid_price,
			));
		} else {
			Self::deposit_event(Event::CancelAuction(auction_id));
		}

		TotalSurplusInAuction::<T>::mutate(|balance| *balance = balance.saturating_sub(surplus_auction.amount));
	}
}

impl<T: Config> AuctionHandler<T::AccountId, Balance, T::BlockNumber, AuctionId> for Pallet<T> {
	fn on_new_bid(
		now: T::BlockNumber,
		id: AuctionId,
		new_bid: (T::AccountId, Balance),
		last_bid: Option<(T::AccountId, Balance)>,
	) -> OnNewBidResult<T::BlockNumber> {
		let bid_result = if <CollateralAuctions<T>>::contains_key(id) {
			Self::collateral_auction_bid_handler(now, id, new_bid, last_bid)
		} else if <DebitAuctions<T>>::contains_key(id) {
			Self::debit_auction_bid_handler(now, id, new_bid, last_bid)
		} else if <SurplusAuctions<T>>::contains_key(id) {
			Self::surplus_auction_bid_handler(now, id, new_bid, last_bid)
		} else {
			Err(Error::<T>::AuctionNotExists.into())
		};

		match bid_result {
			Ok(new_auction_end_time) => OnNewBidResult {
				accept_bid: true,
				auction_end_change: Change::NewValue(Some(new_auction_end_time)),
			},
			Err(_) => OnNewBidResult {
				accept_bid: false,
				auction_end_change: Change::NoChange,
			},
		}
	}

	fn on_auction_ended(id: AuctionId, winner: Option<(T::AccountId, Balance)>) {
		if let Some(collateral_auction) = <CollateralAuctions<T>>::take(id) {
			Self::collateral_auction_end_handler(id, collateral_auction, winner.clone());
		} else if let Some(debit_auction) = <DebitAuctions<T>>::take(id) {
			Self::debit_auction_end_handler(id, debit_auction, winner.clone());
		} else if let Some(surplus_auction) = <SurplusAuctions<T>>::take(id) {
			Self::surplus_auction_end_handler(id, surplus_auction, winner.clone());
		}

		if let Some((bidder, _)) = &winner {
			// decrease account ref of winner
			frame_system::Module::<T>::dec_consumers(bidder);
		}
	}
}

impl<T: Config> AuctionManager<T::AccountId> for Pallet<T> {
	type CurrencyId = CurrencyId;
	type Balance = Balance;
	type AuctionId = AuctionId;

	fn new_collateral_auction(
		refund_recipient: &T::AccountId,
		currency_id: Self::CurrencyId,
		amount: Self::Balance,
		target: Self::Balance,
	) -> DispatchResult {
		ensure!(!amount.is_zero(), Error::<T>::InvalidAmount);
		TotalCollateralInAuction::<T>::try_mutate(currency_id, |total| -> DispatchResult {
			*total = total.checked_add(amount).ok_or(Error::<T>::InvalidAmount)?;
			Ok(())
		})?;

		if !target.is_zero() {
			// no-op if target is zero
			TotalTargetInAuction::<T>::try_mutate(|total| -> DispatchResult {
				*total = total.checked_add(target).ok_or(Error::<T>::InvalidAmount)?;
				Ok(())
			})?;
		}

		let start_time = <frame_system::Module<T>>::block_number();

		// do not set end time for collateral auction
		let auction_id = T::Auction::new_auction(start_time, None)?;

		<CollateralAuctions<T>>::insert(
			auction_id,
			CollateralAuctionItem {
				refund_recipient: refund_recipient.clone(),
				currency_id,
				initial_amount: amount,
				amount,
				target: target,
				start_time,
			},
		);

		// increment recipient account reference
		if frame_system::Module::<T>::inc_consumers(refund_recipient).is_err() {
			// No providers for the locks. This is impossible under normal circumstances
			// since the funds that are under the lock will themselves be stored in the
			// account and therefore will need a reference.
			frame_support::debug::warn!(
				"Warning: Attempt to introduce lock consumer reference, yet no providers. \
				This is unexpected but should be safe."
			);
		}

		Self::deposit_event(Event::NewCollateralAuction(auction_id, currency_id, amount, target));
		Ok(())
	}

	fn new_debit_auction(initial_amount: Self::Balance, fix_debit: Self::Balance) -> DispatchResult {
		ensure!(
			!initial_amount.is_zero() && !fix_debit.is_zero(),
			Error::<T>::InvalidAmount,
		);
		TotalDebitInAuction::<T>::try_mutate(|total| -> DispatchResult {
			*total = total.checked_add(fix_debit).ok_or(Error::<T>::InvalidAmount)?;
			Ok(())
		})?;

		let start_time = <frame_system::Module<T>>::block_number();
		let end_block = start_time + T::AuctionTimeToClose::get();

		// set end time for debit auction
		let auction_id = T::Auction::new_auction(start_time, Some(end_block))?;

		<DebitAuctions<T>>::insert(
			auction_id,
			DebitAuctionItem {
				initial_amount,
				amount: initial_amount,
				fix: fix_debit,
				start_time,
			},
		);

		Self::deposit_event(Event::NewDebitAuction(auction_id, initial_amount, fix_debit));
		Ok(())
	}

	fn new_surplus_auction(amount: Self::Balance) -> DispatchResult {
		ensure!(!amount.is_zero(), Error::<T>::InvalidAmount,);
		TotalSurplusInAuction::<T>::try_mutate(|total| -> DispatchResult {
			*total = total.checked_add(amount).ok_or(Error::<T>::InvalidAmount)?;
			Ok(())
		})?;

		let start_time = <frame_system::Module<T>>::block_number();

		// do not set end time for surplus auction
		let auction_id = T::Auction::new_auction(start_time, None)?;

		<SurplusAuctions<T>>::insert(auction_id, SurplusAuctionItem { amount, start_time });

		Self::deposit_event(Event::NewSurplusAuction(auction_id, amount));
		Ok(())
	}

	fn cancel_auction(id: Self::AuctionId) -> DispatchResult {
		if let Some(collateral_auction) = <CollateralAuctions<T>>::take(id) {
			Self::cancel_collateral_auction(id, collateral_auction)?;
		} else if let Some(debit_auction) = <DebitAuctions<T>>::take(id) {
			Self::cancel_debit_auction(id, debit_auction)?;
		} else if let Some(surplus_auction) = <SurplusAuctions<T>>::take(id) {
			Self::cancel_surplus_auction(id, surplus_auction)?;
		} else {
			return Err(Error::<T>::AuctionNotExists.into());
		}
		T::Auction::remove_auction(id);
		Ok(())
	}

	fn get_total_collateral_in_auction(id: Self::CurrencyId) -> Self::Balance {
		Self::total_collateral_in_auction(id)
	}

	fn get_total_surplus_in_auction() -> Self::Balance {
		Self::total_surplus_in_auction()
	}

	fn get_total_debit_in_auction() -> Self::Balance {
		Self::total_debit_in_auction()
	}

	fn get_total_target_in_auction() -> Self::Balance {
		Self::total_target_in_auction()
	}
}
