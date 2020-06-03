
use crate::auction_proxy::Auction;

use crate::bls_key::*;
use crate::node_state::*;
use crate::user_stake_state::*;
// use crate::util::*;

use crate::events::*;
use crate::node_config::*;
use crate::rewards::*;
use crate::settings::*;
use crate::user_data::*;

imports!();

#[elrond_wasm_derive::module(NodeActivationModuleImpl)]
pub trait ContractStakeModule {

    #[module(UserDataModuleImpl)]
    fn user_data(&self) -> UserDataModuleImpl<T, BigInt, BigUint>;

    #[module(SettingsModuleImpl)]
    fn settings(&self) -> SettingsModuleImpl<T, BigInt, BigUint>;

    #[module(EventsModuleImpl)]
    fn events(&self) -> EventsModuleImpl<T, BigInt, BigUint>;

    #[module(NodeConfigModuleImpl)]
    fn node_config(&self) -> NodeConfigModuleImpl<T, BigInt, BigUint>;

    #[module(RewardsModuleImpl)]
    fn rewards(&self) -> RewardsModuleImpl<T, BigInt, BigUint>;

    #[module(NodeActivationModuleImpl)]
    fn node_activation(&self) -> NodeActivationModuleImpl<T, BigInt, BigUint>;


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
                let node_id = self.node_config().getNodeId(&bls_key);
                node_ids.push(node_id);
                if self.node_config()._get_node_state(node_id) != NodeState::Inactive {
                    return Err("node not inactive");
                }
                self.node_config()._set_node_state(node_id, NodeState::PendingActivation);
            } else {
                // check signature lengths
                let signature = arg;
                if signature.len() != BLS_SIGNATURE_BYTE_LENGTH {
                    return Err("wrong size BLS signature");
                }
            }
        }

        let stake = BigUint::from(num_nodes) * self.node_config().getStakePerNode();
        self.user_data().transform_user_stake_asc(UserStakeState::Inactive, UserStakeState::PendingActivation, &stake)?;

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

        let stake_sent = BigUint::from(node_ids.len()) * self.node_config().getStakePerNode();

        match call_result {
            AsyncCallResult::Ok(()) => {
                // All rewards need to be recalculated now, 
                // because the rewardable stake changes.
                self.rewards().computeAllRewards();

                // set user stake to Active
                self.user_data().transform_user_stake_asc(UserStakeState::PendingActivation, UserStakeState::Active, &stake_sent)?;

                // set nodes to Active
                for &node_id in node_ids.iter() {
                    self.node_config()._set_node_state(node_id, NodeState::Active);
                }

                // log event (no data)
                self.events().activation_ok_event(());
            },
            AsyncCallResult::Err(error) => {
                // revert user stake to Inactive
                self.user_data().transform_user_stake_asc(UserStakeState::PendingActivation, UserStakeState::Inactive, &stake_sent)?;

                // revert nodes to Inactive
                for &node_id in node_ids.iter() {
                    self.node_config()._set_node_state(node_id, NodeState::Inactive);
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
            #[var_args] bls_keys: Vec<BLSKey>) -> Result<(), &str> {

        if self.get_caller() != self.settings().getContractOwner() {
            return Err("only owner can deactivate"); 
        }

        let mut node_ids = Vec::<usize>::with_capacity(bls_keys.len());
        for bls_key in bls_keys.iter() {
            let node_id = self.node_config().getNodeId(&bls_key);
            node_ids.push(node_id);
            if self.node_config()._get_node_state(node_id) != NodeState::Active {
                return Err("node not active");
            }
            self.node_config()._set_node_state(node_id, NodeState::PendingDeactivation);
        }

        self._perform_deactivate_nodes(None, node_ids, bls_keys)
    }

    #[private]
    fn _perform_deactivate_nodes(&self,
            opt_requester_id: Option<usize>,
            node_ids: Vec<usize>,
            bls_keys: Vec<BLSKey>) -> Result<(), &str> {

        // All rewards need to be recalculated now, 
        // because the rewardable stake will change shortly.
        self.rewards().computeAllRewards();

        // convert user stake to PendingDeactivation
        let mut stake_to_deactivate = BigUint::from(bls_keys.len()) * self.node_config().getStakePerNode();
        if let Some(requester_id) = opt_requester_id {
            // if requested by a user, that user has priority
            stake_to_deactivate = self.user_data().transform_user_stake(
                requester_id,
                UserStakeState::Active, UserStakeState::PendingDeactivation,
                stake_to_deactivate);
        }
        self.user_data().transform_user_stake_desc(
            UserStakeState::Active, UserStakeState::PendingDeactivation,
            &stake_to_deactivate)?;

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

        let stake_sent = BigUint::from(node_ids.len()) * self.node_config().getStakePerNode();

        match call_result {
            AsyncCallResult::Ok(()) => {
                // set user stake to Active
                self.user_data().transform_user_stake_desc(UserStakeState::PendingDeactivation, UserStakeState::UnBondPeriod, &stake_sent)?;

                // set nodes to Active
                for &node_id in node_ids.iter() {
                    self.node_config()._set_node_state(node_id, NodeState::UnBondPeriod);
                }

                // log event (no data)
                self.events().deactivation_ok_event(());
            },
            AsyncCallResult::Err(error) => {
                // revert user stake to Inactive
                self.user_data().transform_user_stake_desc(UserStakeState::PendingDeactivation, UserStakeState::Active, &stake_sent)?;

                // revert nodes to Inactive
                for &node_id in node_ids.iter() {
                    self.node_config()._set_node_state(node_id, NodeState::Active);
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
            #[var_args] bls_keys: Vec<BLSKey>) -> Result<(), &str> {

        let mut node_ids = Vec::<usize>::with_capacity(bls_keys.len());
        for bls_key in bls_keys.iter() {
            let node_id = self.node_config().getNodeId(&bls_key);
            node_ids.push(node_id);
            if self.node_config()._get_node_state(node_id) != NodeState::UnBondPeriod {
                return Err("node not in unbond period");
            }
            self.node_config()._set_node_state(node_id, NodeState::PendingUnBond);
        }

        let stake = BigUint::from(bls_keys.len()) * self.node_config().getStakePerNode();
        self.user_data().transform_user_stake_desc(UserStakeState::UnBondPeriod, UserStakeState::PendingUnBond, &stake)?;
        
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

        let stake_sent = BigUint::from(node_ids.len()) * self.node_config().getStakePerNode();

        match call_result {
            AsyncCallResult::Ok(()) => {
                // set user stake to Active
                // TODO: make sure delegators with stake for sale get the stake first
                self.user_data().transform_user_stake_desc(UserStakeState::PendingUnBond, UserStakeState::Inactive, &stake_sent)?;

                // set nodes to Inactive
                for &node_id in node_ids.iter() {
                    self.node_config()._set_node_state(node_id, NodeState::Inactive);
                }

                // log event (no data)
                self.events().unBond_ok_event(());
            },
            AsyncCallResult::Err(error) => {
                // revert user stake to Inactive
                self.user_data().transform_user_stake_desc(UserStakeState::PendingUnBond, UserStakeState::UnBondPeriod, &stake_sent)?;

                // revert nodes to Inactive
                for &node_id in node_ids.iter() {
                    self.node_config()._set_node_state(node_id, NodeState::UnBondPeriod);
                }

                // log failure event (no data)
                self.events().unBond_fail_event(error.err_msg);
            }
        }

        Ok(())
    }

    /// Delegators can force some or all nodes to unstake
    /// if they put up stake for sale and no-one has bought it for long enough.
    /// This operation can be performed by any delegator.
    fn forceUnstake(&self) -> Result<(), &str> {
        let user_id = self.user_data().getUserId(&self.get_caller());
        if user_id == 0 {
            return Err("only delegators can call forceUnstake");
        }

        let stake_for_sale = self.user_data()._get_user_stake_for_sale(user_id);
        if stake_for_sale == 0 {
            return Err("only delegators that are trying to sell stake can call forceUnstake");
        }

        let time_of_stake_offer = self.user_data()._get_user_time_of_stake_offer(user_id);
        let time_before_force_unstake = self.settings().getTimeBeforeForceUnstake();
        if self.get_block_timestamp() <= time_of_stake_offer + time_before_force_unstake {
            return Err("too soon to call forceUnstake");
        }

        // find enough nodes to cover requested stake
        let mut node_ids: Vec<usize> = Vec::new();
        let mut bls_keys: Vec<BLSKey> = Vec::new();
        let mut i = self.node_config().getNumNodes();
        let mut node_stake = BigUint::zero();
        let stake_per_node = self.node_config().getStakePerNode();
        while i > 0 && stake_for_sale > node_stake {
            if let NodeState::Active = self.node_config()._get_node_state(i) {
                node_stake += &stake_per_node;
                node_ids.push(i);
                bls_keys.push(self.node_config()._get_node_id_to_bls(i));
            }
            i -= 1;
        }
 
        self._perform_deactivate_nodes(Some(user_id), node_ids, bls_keys)
    }

}
