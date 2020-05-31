use crate::node_state::*;

use crate::events::*;
use crate::user_data::*;

imports!();

/// Deals with stake trade among delegators.
/// Note: each 1 staked ERD can only be traded for 1 unstaked ERD.
#[elrond_wasm_derive::module(StakeSaleModuleImpl)]
pub trait StakeSaleModule {

    #[module(EventsModuleImpl)]
    fn events(&self) -> EventsModuleImpl<T, BigInt, BigUint>;

    #[module(UserDataModuleImpl)]
    fn user_data(&self) -> UserDataModuleImpl<T, BigInt, BigUint>;



    /// Creates a stake offer. Overwrites any previous stake offer.
    /// Once a stake offer is up, it can be bought by anyone on a first come first served basis.
    fn offerStakeForSale(&self, amount: BigUint) -> Result<(), &str> {
        let caller = self.get_caller();
        let user_id = self.user_data().getUserId(&caller);
        if user_id == 0 {
            return Err("only delegators can offer stake for sale")
        }

        // get active stake
        let stake = self.user_data()._get_user_stake_of_type(user_id, NodeState::Active);
        if &amount > &stake {
            return Err("cannot offer more than the user active stake")
        }

        // store offer
        self.user_data()._set_user_stake_for_sale(user_id, &amount);
        self.user_data()._set_user_time_of_stake_offer(user_id, self.get_block_timestamp());

        Ok(())
    }

    /// Check if a user is willing to sell stake, and how much.
    #[view]
    fn getStakeForSale(&self, user: Address) -> BigUint {
        let user_id = self.user_data().getUserId(&user);
        if user_id == 0 {
            return BigUint::zero()
        }
        self.user_data()._get_user_stake_for_sale(user_id)
    }

    /// User-to-user purchase of stake.
    /// Only stake that has been offered for sale by owner can be bought.
    /// Note: the price of 1 staked ERD must always be 1 "free" ERD, from outside the contract.
    /// The payment for the stake does not stay in the contract, it gets forwarded immediately to the seller.
    #[payable]
    fn purchaseStake(&self, seller: Address, #[payment] payment: BigUint) -> Result<(), &str> {
        if payment == 0 {
            return Ok(())
        }

        // get seller id
        let seller_id = self.user_data().getUserId(&seller);
        if seller_id == 0 {
            return Err("unknown seller")
        }

        // decrease stake for sale
        self.user_data()._update_user_stake_for_sale(seller_id, |stake_for_sale| {
            if &payment > &*stake_for_sale {
                Err("payment exceeds stake offered")
            } else {
                *stake_for_sale -= &payment;
                Ok(())
            }
        })?;

        // decrease stake of seller
        self.user_data()._update_user_active_stake(seller_id, |seller_active_stake| {
            if &payment > &*seller_active_stake {
                Err("payment exceeds seller active stake")
            } else {
                *seller_active_stake -= &payment;
                Ok(())
            }
        })?;
        self.user_data()._update_user_total_stake(seller_id, |seller_total_stake| {
            *seller_total_stake -= &payment;
        });

        // get buyer id or create buyer
        let caller = self.get_caller();
        let mut buyer_id = self.user_data().getUserId(&caller);
        if buyer_id == 0 {
            buyer_id = self.user_data().new_user();
            self.user_data()._set_user_id(&caller, buyer_id);
        }

        // increase stake of buyer
        self.user_data()._update_user_total_stake(buyer_id, |buyer_total_stake| {
            *buyer_total_stake += &payment;
        });
        self.user_data()._update_user_active_stake(buyer_id, |buyer_active_stake| {
            *buyer_active_stake += &payment;
        });

        // forward payment to seller
        self.send_tx(&seller, &payment, "payment for stake");

        // log transaction
        self.events().purchase_stake_event(&seller, &caller, &payment);

        Ok(())
    }
}