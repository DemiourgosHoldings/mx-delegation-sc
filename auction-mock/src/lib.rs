#![no_std]
#![allow(non_snake_case)]
#![allow(unused_attributes)]

mod storage;
use storage::*;

#[cfg(feature = "node-storage-default")]
pub use node_storage_default as node_storage;
#[cfg(feature = "node-storage-wasm")]
pub use node_storage_wasm as node_storage;

elrond_wasm::imports!();

use node_storage::types::bls_key::*;

#[elrond_wasm_derive::contract(AuctionMockImpl)]
pub trait AuctionMock {
    #[module(AuctionMockStorageImpl)]
    fn storage(&self) -> AuctionMockStorageImpl<T, BigInt, BigUint>;

    #[init]
    fn init(&self) {}

    #[payable("EGLD")]
    #[endpoint]
    fn stake(
        &self,
        num_nodes: usize,
        #[var_args] bls_keys_signatures_args: VarArgs<MultiArg2<BoxedBytes, BoxedBytes>>,
        #[payment] payment: &BigUint,
    ) -> SCResult<MultiResultVec<BoxedBytes>> {
        let bls_keys_signatures = bls_keys_signatures_args.into_vec();
        require!(
            num_nodes == bls_keys_signatures.len(),
            "incorrect number of arguments"
        );

        require!(
            !self.storage().is_staking_failure(),
            "auction smart contract deliberate error"
        );

        let mut new_num_nodes = self.storage().get_num_nodes();
        let expected_payment = BigUint::from(num_nodes) * self.storage().get_stake_per_node();
        require!(
            payment == &expected_payment,
            "incorrect payment to auction mock"
        );

        let mut result_err_data: Vec<BoxedBytes> = Vec::new();
        for key_sig_pair in bls_keys_signatures.into_iter() {
            new_num_nodes += 1;
            let (bls_key, bls_sig) = key_sig_pair.into_tuple();
            self.storage()
                .set_stake_bls_key(new_num_nodes, bls_key.as_slice());
            self.storage()
                .set_stake_bls_signature(new_num_nodes, bls_sig.as_slice());

            let err_code = self.storage().get_bls_deliberate_error(bls_key.as_slice());
            if err_code > 0 {
                result_err_data.push(bls_key);
                result_err_data.push(BoxedBytes::from(&[err_code][..]));
            }
        }

        self.storage().set_num_nodes(new_num_nodes);

        Ok(result_err_data.into())
    }

    #[endpoint(unStake)]
    fn unstake_endpoint(
        &self,
        #[var_args] bls_keys: VarArgs<Vec<u8>>,
    ) -> SCResult<MultiResultVec<Vec<u8>>> {
        require!(
            !self.storage().is_staking_failure(),
            "auction smart contract deliberate error"
        );

        let mut result_err_data: Vec<Vec<u8>> = Vec::new();
        for (n, bls_key) in bls_keys.iter().enumerate() {
            self.storage().set_unStake_bls_key(n, bls_key);

            let err_code = self.storage().get_bls_deliberate_error(bls_key);
            if err_code > 0 {
                result_err_data.push(bls_key.clone());
                result_err_data.push([err_code].to_vec());
            }
        }

        Ok(result_err_data.into())
    }

    #[endpoint(unStakeNodes)]
    fn unstake_nodes_endpoint(
        &self,
        #[var_args] bls_keys: VarArgs<Vec<u8>>,
    ) -> SCResult<MultiResultVec<Vec<u8>>> {
        self.unstake_endpoint(bls_keys)
    }

    #[endpoint(unBond)]
    fn unbond_endpoint(
        &self,
        #[var_args] bls_keys: VarArgs<Vec<u8>>,
    ) -> SCResult<MultiResultVec<Vec<u8>>> {
        require!(
            !self.storage().is_staking_failure(),
            "auction smart contract deliberate error"
        );

        let mut result_err_data: Vec<Vec<u8>> = Vec::new();
        for (n, bls_key) in bls_keys.iter().enumerate() {
            self.storage().set_unBond_bls_key(n, bls_key);

            let err_code = self.storage().get_bls_deliberate_error(bls_key);
            if err_code > 0 {
                result_err_data.push(bls_key.clone());
                result_err_data.push([err_code].to_vec());
            }
        }

        let unbond_stake = BigUint::from(bls_keys.len()) * self.storage().get_stake_per_node();
        self.send()
            .direct_egld(&self.get_caller(), &unbond_stake, b"unbond stake");

        Ok(result_err_data.into())
    }

    #[endpoint]
    fn claim(&self) -> SCResult<()> {
        Ok(())
    }

    #[payable("EGLD")]
    #[endpoint(unJail)]
    fn unjail_endpoint(
        &self,
        #[var_args] bls_keys: VarArgs<BLSKey>,
        #[payment] _fine_payment: BigUint,
    ) -> SCResult<()> {
        self.storage().set_unjailed(&bls_keys.into_vec());
        Ok(())
    }
}
