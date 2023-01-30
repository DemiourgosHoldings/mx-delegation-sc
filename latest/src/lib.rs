#![no_std]
#![allow(clippy::string_lit_as_bytes)]
#![allow(clippy::type_complexity)]

// auxiliaries
pub mod auction_proxy;

// modules
pub mod events;
pub mod node_activation;
pub mod reset_checkpoint_endpoints;
pub mod reset_checkpoint_state;
pub mod reset_checkpoint_types;
pub mod rewards_endpoints;
pub mod rewards_state;
pub mod settings;
pub mod user_stake_dust_cleanup;
pub mod user_stake_endpoints;
pub mod user_stake_state;

pub use multiversx_sc_modules;
pub use node_storage;
pub use user_fund_storage;
