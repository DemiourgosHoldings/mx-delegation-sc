
use crate::auction_proxy::Auction;

use crate::bls_key::*;
use crate::node_state::*;
// use crate::util::*;

use crate::events::*;
use crate::nodes::*;
use crate::rewards::*;
use crate::settings::*;
use crate::user_data::*;

imports!();

#[elrond_wasm_derive::module(ContractStakeModuleImpl)]
pub trait ContractStakeModule {

    #[module(UserDataModuleImpl)]
    fn user_data(&self) -> UserDataModuleImpl<T, BigInt, BigUint>;

    #[module(SettingsModuleImpl)]
    fn settings(&self) -> SettingsModuleImpl<T, BigInt, BigUint>;

    #[module(EventsModuleImpl)]
    fn events(&self) -> EventsModuleImpl<T, BigInt, BigUint>;

    #[module(NodeModuleImpl)]
    fn nodes(&self) -> NodeModuleImpl<T, BigInt, BigUint>;

    #[module(RewardsModuleImpl)]
    fn rewards(&self) -> RewardsModuleImpl<T, BigInt, BigUint>;

    #[module(ContractStakeModuleImpl)]
    fn contract_stake(&self) -> ContractStakeModuleImpl<T, BigInt, BigUint>;


    /// Yields how much stake was added to the contract.
    #[view]
    #[storage_get("filled_stake")]
    fn getFilledStake(&self) -> BigUint;

    #[private]
    #[storage_set("filled_stake")]
    fn _set_filled_stake(&self, filled_stake: &BigUint);

    /// This is stake that is in the contract, not sent to the auction contract.
    #[view]
    #[storage_get("total_active_stake")]
    fn getTotalActiveStake(&self) -> BigUint;

    #[private]
    #[storage_set("total_active_stake")]
    fn _set_total_active_stake(&self, total_active_stake: &BigUint);

    #[private]
    fn transform_user_stake(&self, user_id: usize, old_type: NodeState, new_type: NodeState, mut total_supply: BigUint) -> BigUint {
        let mut user_stake_old_type = self.user_data()._get_user_stake_of_type(user_id, old_type);
        let mut user_stake_new_type = self.user_data()._get_user_stake_of_type(user_id, new_type);
        if total_supply > user_stake_old_type {
            user_stake_new_type += &user_stake_old_type;
            total_supply -= &user_stake_old_type;
            user_stake_old_type = BigUint::zero();
        } else {
            user_stake_old_type -= &total_supply;
            user_stake_new_type += &total_supply;
            total_supply = BigUint::zero();
        }
        self.user_data()._set_user_stake_of_type(user_id, old_type, &user_stake_old_type);
        self.user_data()._set_user_stake_of_type(user_id, new_type, &user_stake_new_type);
        
        total_supply
    }

    /// Converts inactive stake into active stake for users.
    /// Walking in increasing user id order, so older users get picked first.
    #[private]
    fn transform_user_stake_asc(&self, old_type: NodeState, new_type: NodeState, amount: &BigUint) -> Result<(), &'static str> {
        let mut remaining = amount.clone();
        let num_users = self.user_data().getNumUsers();
        let mut i = 1usize;
        while i <= num_users && remaining > 0 {
            remaining = self.transform_user_stake(i, old_type, new_type, remaining);
            i += 1;
        }

        if remaining > 0 {
            Err("not enough user stake")
        } else {
            Ok(())
        }
    }

    #[private]
    fn transform_user_stake_desc(&self, old_type: NodeState, new_type: NodeState, amount: &BigUint) -> Result<(), &'static str> {
        let mut remaining = amount.clone();
        let mut i = self.user_data().getNumUsers();
        while i > 0 && remaining > 0 {
            remaining = self.transform_user_stake(i, old_type, new_type, remaining);
            i -= 1;
        }

        if remaining > 0 {
            Err("not enough active stake")
        } else {
            Ok(())
        }
    }


    /// Send stake to the staking contract, if the entire stake has been gathered.
    fn activateNodes(&self,
            num_nodes: usize,
            #[multi(2*num_nodes)] bls_keys_signatures: Vec<Vec<u8>>)
        -> Result<(), &str> {

        if self.get_caller() != self.settings().getContractOwner() {
            return Err("only owner can activate"); 
        }

        let mut node_ids = Vec::<usize>::with_capacity(num_nodes);
        for (i, arg) in bls_keys_signatures.iter().enumerate() {
            if i % 2 == 0 {
                // set nodes to active & collect ids
                let bls_key = BLSKey::from_bytes(arg)?;
                let node_id = self.nodes().getNodeId(&bls_key);
                node_ids.push(node_id);
                if self.nodes().getNodeState(node_id) != NodeState::Inactive {
                    return Err("node not inactive");
                }
                self.nodes()._set_node_state(node_id, NodeState::PendingActivation);
            } else {
                // check signature lengths
                let signature = arg;
                if signature.len() != BLS_SIGNATURE_BYTE_LENGTH {
                    return Err("wrong size BLS signature");
                }
            }
        }

        let stake = BigUint::from(num_nodes) * self.nodes().getStakePerNode();
        self.transform_user_stake_asc(NodeState::Inactive, NodeState::PendingActivation, &stake)?;

        // send all stake to auction contract
        let auction_contract_addr = self.settings().getAuctionContractAddress();
        let auction_contract = contract_proxy!(self, &auction_contract_addr, Auction);
        auction_contract.stake(
            node_ids, // callback arg
            num_nodes,
            bls_keys_signatures,
            &stake);

        Ok(())
    }

    /// Only finalize activation if we got confirmation from the auction contract.
    /// #[callback] can only be declared in lib.rs for the moment.
    #[private]
    fn auction_stake_callback(&self, 
            node_ids: Vec<usize>, // #[callback_arg]
            call_result: AsyncCallResult<()>) -> Result<(), &str> {

        let stake_sent = BigUint::from(node_ids.len()) * self.nodes().getStakePerNode();

        match call_result {
            AsyncCallResult::Ok(()) => {
                // All rewards need to be recalculated now, 
                // because the rewardable stake changes.
                self.rewards().computeAllRewards();

                // set user stake to Active
                self.transform_user_stake_asc(NodeState::PendingActivation, NodeState::Active, &stake_sent)?;

                // also increase total active stake, required for rewards
                let mut total_active_stake = self.contract_stake().getTotalActiveStake();
                total_active_stake += &stake_sent;
                self.contract_stake()._set_total_active_stake(&total_active_stake);

                // set nodes to Active
                for &node_id in node_ids.iter() {
                    self.nodes()._set_node_state(node_id, NodeState::Active);
                }

                // log event (no data)
                self.events().activation_ok_event(());
            },
            AsyncCallResult::Err(error) => {
                // revert user stake to Inactive
                self.transform_user_stake_asc(NodeState::PendingActivation, NodeState::Inactive, &stake_sent)?;

                // revert nodes to Inactive
                for &node_id in node_ids.iter() {
                    self.nodes()._set_node_state(node_id, NodeState::Inactive);
                }

                // log failure event (no data)
                self.events().activation_fail_event(error.err_msg);
            }
        }

        Ok(())
    }


    // DEACTIVATE + FORCE UNSTAKE

    /// Unstakes from the auction smart contract.
    /// The contract will stop receiving rewards, but stake cannot be yet reclaimed.
    /// This operation is performed by the owner.
    fn deactivateNodes(&self,
            bls_keys: Vec<BLSKey>) -> Result<(), &str> {

        if self.get_caller() != self.settings().getContractOwner() {
            return Err("only owner can deactivate"); 
        }

        // All rewards need to be recalculated now, 
        // because the rewardable stake will change shortly.
        self.rewards().computeAllRewards();

        let mut node_ids = Vec::<usize>::with_capacity(bls_keys.len());
        for bls_key in bls_keys.iter() {
            let node_id = self.nodes().getNodeId(&bls_key);
            node_ids.push(node_id);
            if self.nodes().getNodeState(node_id) != NodeState::Active {
                return Err("node not active");
            }
            self.nodes()._set_node_state(node_id, NodeState::PendingActivation);
        }

        let stake = BigUint::from(bls_keys.len()) * self.nodes().getStakePerNode();
        self.transform_user_stake_asc(NodeState::Active, NodeState::PendingDeactivation, &stake)?;

        // also decrease total active stake, required for rewards
        let mut total_active_stake = self.contract_stake().getTotalActiveStake();
        total_active_stake -= &stake;
        self.contract_stake()._set_total_active_stake(&total_active_stake);

        // self._perform_deactivate()

        // send unstake command to Auction SC
        let auction_contract_addr = self.settings().getAuctionContractAddress();
        let auction_contract = contract_proxy!(self, &auction_contract_addr, Auction);
        auction_contract.unStake(
            node_ids,
            bls_keys);

        Ok(())
    }

    /// Only finalize deactivation if we got confirmation from the auction contract.
    /// #[callback] can only be declared in lib.rs for the moment.
    #[private]
    fn auction_unStake_callback(&self, 
            node_ids: Vec<usize>, // #[callback_arg]
            call_result: AsyncCallResult<()>) -> Result<(), &str> {

        let stake_sent = BigUint::from(node_ids.len()) * self.nodes().getStakePerNode();

        match call_result {
            AsyncCallResult::Ok(()) => {
                // set user stake to Active
                self.transform_user_stake_asc(NodeState::PendingDeactivation, NodeState::UnBondPeriod, &stake_sent)?;

                // set nodes to Active
                for &node_id in node_ids.iter() {
                    self.nodes()._set_node_state(node_id, NodeState::UnBondPeriod);
                }

                // log event (no data)
                self.events().deactivation_ok_event(());
            },
            AsyncCallResult::Err(error) => {
                // revert user stake to Inactive
                self.transform_user_stake_asc(NodeState::PendingDeactivation, NodeState::Active, &stake_sent)?;

                // revert nodes to Inactive
                for &node_id in node_ids.iter() {
                    self.nodes()._set_node_state(node_id, NodeState::Active);
                }

                // log failure event (no data)
                self.events().deactivation_fail_event(error.err_msg);
            }
        }

        Ok(())
    }

    // UNBOND

    /// Claims unstaked stake from the auction smart contract.
    /// This operation can be executed by anyone (note that it might cost much gas).
    fn unBond(&self,
            bls_keys: Vec<BLSKey>) -> Result<(), &str> {

        let mut node_ids = Vec::<usize>::with_capacity(bls_keys.len());
        for bls_key in bls_keys.iter() {
            let node_id = self.nodes().getNodeId(&bls_key);
            node_ids.push(node_id);
            if self.nodes().getNodeState(node_id) != NodeState::Active {
                return Err("node not in unbond period");
            }
            self.nodes()._set_node_state(node_id, NodeState::PendingUnBond);
        }
        
        // send unbond command to Auction SC
        let auction_contract_addr = self.settings().getAuctionContractAddress();
        let auction_contract = contract_proxy!(self, &auction_contract_addr, Auction);
        auction_contract.unBond(
            node_ids,
            bls_keys);

        Ok(())
    }

    /// Only finalize deactivation if we got confirmation from the auction contract.
    /// #[callback] can only be declared in lib.rs for the moment.
    #[private]
    fn auction_unBond_callback(&self,
            node_ids: Vec<usize>, // #[callback_arg]
            call_result: AsyncCallResult<()>) -> Result<(), &str> {

        let stake_sent = BigUint::from(node_ids.len()) * self.nodes().getStakePerNode();

        match call_result {
            AsyncCallResult::Ok(()) => {
                // set user stake to Active
                self.transform_user_stake_asc(NodeState::PendingUnBond, NodeState::Inactive, &stake_sent)?;

                // set nodes to Inactive
                for &node_id in node_ids.iter() {
                    self.nodes()._set_node_state(node_id, NodeState::Inactive);
                }

                // log event (no data)
                self.events().unBond_ok_event(());
            },
            AsyncCallResult::Err(error) => {
                // revert user stake to Inactive
                self.transform_user_stake_asc(NodeState::PendingUnBond, NodeState::UnBondPeriod, &stake_sent)?;

                // revert nodes to Inactive
                for &node_id in node_ids.iter() {
                    self.nodes()._set_node_state(node_id, NodeState::UnBondPeriod);
                }

                // log failure event (no data)
                self.events().unBond_fail_event(error.err_msg);
            }
        }

        Ok(())
    }

        // /// Delegators can force the entire contract to unstake
    // /// if they put up stake for sale and no-one has bought it for long enough.
    // /// This operation can be performed by any delegator.
    // fn forceUnstake(&self) -> Result<(), &str> {
    //     let user_id = self.user_data().getUserId(&self.get_caller());
    //     if user_id == 0 {
    //         return Err("only delegators can call forceUnstake");
    //     }

    //     if self.user_data()._get_user_stake_for_sale(user_id) == 0 {
    //         return Err("only delegators that are trying to sell stake can call forceUnstake");
    //     }

    //     let time_of_stake_offer = self.user_data()._get_user_time_of_stake_offer(user_id);
    //     let time_before_force_unstake = self.settings().getTimeBeforeForceUnstake();
    //     if self.get_block_timestamp() <= time_of_stake_offer + time_before_force_unstake {
    //         return Err("too soon to call forceUnstake");
    //     }


 
    //     self._perform_deactivate()
    // }

    // #[private]
    // fn _perform_deactivate(&self) -> Result<(), &str> {
    //     // change state
    //     self._set_stake_state(NodeState::PendingDeactivation);
        
    //     // send unstake command to Auction SC
    //     let auction_contract_addr = self.settings().getAuctionContractAddress();
    //     let auction_contract = contract_proxy!(self, &auction_contract_addr, Auction);
    //     auction_contract.unStake(self.nodes().getBlsKeys());

    //     Ok(())
    // }

}
