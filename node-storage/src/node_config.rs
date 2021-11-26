use crate::types::{BLSKey, BLSSignature, BLSStatusMultiArg, NodeState};

elrond_wasm::imports!();

pub const MAX_NODES_PER_OPERATION: usize = 100;

pub type NodeIndexArrayVec = ArrayVec<usize, MAX_NODES_PER_OPERATION>;

/// Indicates how we express the percentage of rewards that go to the node.
/// Since we cannot have floating point numbers, we use fixed point with this denominator.
/// Percents + 2 decimals -> 10000.
// pub static PERCENTAGE_DENOMINATOR: usize = 10000;

/// This module manages the validator node info:
/// - how many nodes there are,
/// - what BLS keys they have.
/// - what BLS signatures they have
///
#[elrond_wasm::derive::module]
pub trait NodeConfigModule {
    /// The number of nodes that will run with the contract stake, as configured by the owner.
    #[view(getNumNodes)]
    #[storage_mapper("num_nodes")]
    fn num_nodes(&self) -> SingleValueMapper<usize>;

    /// Each node gets a node id. This is in order to be able to iterate over their data.
    /// This is a mapping from node BLS key to node id.
    /// The key is the bytes "node_id" concatenated with the BLS key. The value is the node id.
    /// Ids start from 1 because 0 means unset of None.
    #[view(getNodeId)]
    #[storage_get("node_bls_to_id")]
    fn get_node_id(&self, bls_key: &BLSKey<Self::Api>) -> usize;

    #[storage_set("node_bls_to_id")]
    fn set_node_bls_to_id(&self, bls_key: &BLSKey<Self::Api>, node_id: usize);

    #[storage_get("node_id_to_bls")]
    fn get_node_id_to_bls(&self, node_id: usize) -> BLSKey<Self::Api>;

    #[storage_set("node_id_to_bls")]
    fn set_node_id_to_bls(&self, node_id: usize, bls_key: &BLSKey<Self::Api>);

    #[storage_get("node_signature")]
    fn get_node_signature(&self, node_id: usize) -> BLSSignature<Self::Api>;

    #[storage_set("node_signature")]
    fn set_node_signature(&self, node_id: usize, node_signature: BLSSignature<Self::Api>);

    #[view(getNodeSignature)]
    fn get_node_signature_endpoint(
        &self,
        bls_key: BLSKey<Self::Api>,
    ) -> OptionalResult<BLSSignature<Self::Api>> {
        let node_id = self.get_node_id(&bls_key);
        if node_id == 0 {
            OptionalResult::None
        } else {
            OptionalResult::Some(self.get_node_signature(node_id))
        }
    }

    /// Current state of node: inactive, active, deleted, etc.
    #[storage_get("node_state")]
    fn get_node_state(&self, node_id: usize) -> NodeState;

    #[storage_set("node_state")]
    fn set_node_state(&self, node_id: usize, node_state: NodeState);

    #[view(getNodeState)]
    fn get_node_state_endpoint(&self, bls_key: BLSKey<Self::Api>) -> NodeState {
        let node_id = self.get_node_id(&bls_key);
        if node_id == 0 {
            NodeState::Removed
        } else {
            self.get_node_state(node_id)
        }
    }

    #[view(getAllNodeStates)]
    fn get_all_node_states(&self) -> MultiResultVec<MultiResult2<BLSKey<Self::Api>, u8>> {
        let num_nodes = self.num_nodes().get();
        let mut result = Vec::new();
        for i in 1..num_nodes + 1 {
            result.push(MultiResult2::from((
                self.get_node_id_to_bls(i),
                self.get_node_state(i).discriminant(),
            )));
        }
        result.into()
    }

    #[view(getNodeBlockNonceOfUnstake)]
    fn get_node_bl_nonce_of_unstake_endpoint(
        &self,
        bls_key: BLSKey<Self::Api>,
    ) -> OptionalResult<u64> {
        let node_id = self.get_node_id(&bls_key);
        if node_id == 0 {
            OptionalResult::None
        } else if let NodeState::UnBondPeriod { started } = self.get_node_state(node_id) {
            OptionalResult::Some(started)
        } else {
            OptionalResult::None
        }
    }

    #[endpoint(addNodes)]
    fn add_nodes(
        &self,
        #[var_args] bls_keys_signatures: ManagedVarArgs<
            MultiArg2<BLSKey<Self::Api>, BLSSignature<Self::Api>>,
        >,
    ) -> SCResult<()> {
        only_owner!(self, "only owner can add nodes");

        let mut num_nodes = self.num_nodes().get();
        for bls_sig_pair_arg in bls_keys_signatures.into_iter() {
            let (bls_key, bls_sig) = bls_sig_pair_arg.into_tuple();
            let mut node_id = self.get_node_id(&bls_key);
            if node_id == 0 {
                num_nodes += 1;
                node_id = num_nodes;
                self.set_node_bls_to_id(&bls_key, node_id);
                self.set_node_id_to_bls(node_id, &bls_key);
                self.set_node_state(node_id, NodeState::Inactive);
                self.set_node_signature(node_id, bls_sig);
            } else if self.get_node_state(node_id) == NodeState::Removed {
                self.set_node_state(node_id, NodeState::Inactive);
                self.set_node_signature(node_id, bls_sig);
            } else {
                return sc_error!("node already registered");
            }
        }
        self.num_nodes().set(&num_nodes);
        Ok(())
    }

    #[endpoint(removeNodes)]
    fn remove_nodes(
        &self,
        #[var_args] bls_keys: ManagedVarArgs<BLSKey<Self::Api>>,
    ) -> SCResult<()> {
        only_owner!(self, "only owner can remove nodes");

        for bls_key in bls_keys.into_iter() {
            let node_id = self.get_node_id(&bls_key);
            require!(node_id != 0, "node not registered");
            require!(
                self.get_node_state(node_id) == NodeState::Inactive,
                "only inactive nodes can be removed"
            );
            self.set_node_state(node_id, NodeState::Removed);
        }

        Ok(())
    }

    fn split_node_ids_by_err(
        &self,
        mut node_ids: NodeIndexArrayVec,
        node_status_args: ManagedVarArgs<BLSStatusMultiArg<Self::Api>>,
    ) -> (NodeIndexArrayVec, NodeIndexArrayVec) {
        let mut failed_node_ids: NodeIndexArrayVec = NodeIndexArrayVec::new();
        for arg in node_status_args.into_iter() {
            let (bls_key, status) = arg.into_tuple();
            if status != 0 {
                let node_id = self.get_node_id(&bls_key);
                // move node from ok nodes to failed ones
                if let Some(pos) = node_ids.iter().position(|x| *x == node_id) {
                    node_ids.swap_remove(pos);
                    failed_node_ids.push(node_id);
                }
            }
        }

        (node_ids, failed_node_ids)
    }
}
