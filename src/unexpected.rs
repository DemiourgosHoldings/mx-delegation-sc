use crate::rewards::*;
use crate::settings::*;
use crate::stake_per_node::*;

imports!();

/// Contains logic for the owner to extract any unexpected balance that resides in the contract.
#[elrond_wasm_derive::module(UnexpectedBalanceModuleImpl)]
pub trait UnexpectedBalanceModule {

    #[module(RewardsModuleImpl)]
    fn rewards(&self) -> RewardsModuleImpl<T, BigInt, BigUint>;

    #[module(SettingsModuleImpl)]
    fn settings(&self) -> SettingsModuleImpl<T, BigInt, BigUint>;

    #[module(ContractStakeModuleImpl)]
    fn contract_stake(&self) -> ContractStakeModuleImpl<T, BigInt, BigUint>;



    /// Expected balance includes:
    /// - stake
    /// - unclaimed rewards (bulk uncomputed rewards + computed but unclaimed rewards; everything that was not yet sent to the delegators).
    /// Everything else is unexpected and can be withdrawn by the owner.
    /// This can come from someone accidentally sending ERD to the contract via direct transfer.
    #[view]
    fn getUnexpectedBalance(&self) -> BigUint {
        let mut expected_balance = self.contract_stake()._get_inactive_stake();
        expected_balance += self.rewards().getTotalCumulatedRewards();
        expected_balance -= self.rewards()._get_sent_rewards();

        self.get_own_balance() - expected_balance
    }

    /// Used by owner to extract unexpected balance from contract.
    fn withdrawUnexpectedBalance(&self) -> Result<(), &str> {
        let caller = self.get_caller();
        if &caller != &self.settings().getContractOwner() {
            return Err("only owner can withdraw unexpected balance");
        }

        let unexpected_balance = self.getUnexpectedBalance();
        if unexpected_balance > 0 {
            self.send_tx(&caller, &unexpected_balance, "unexpected balance");
        }

        Ok(())
    }


}
