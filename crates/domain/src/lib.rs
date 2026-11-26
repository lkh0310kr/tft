//! TFT domain data and invariants. This crate contains no application UI.

pub mod effect;
pub mod vault;

pub use effect::VaultEvent;
pub use vault::{
    BackendInstanceId, BackendObjectId, FileMeta, InvariantError, LookupError, NameConflict,
    NodeId, NodeKind, RenameError, ScannedNode, VaultData, VaultNode,
};
