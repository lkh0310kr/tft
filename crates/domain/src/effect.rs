//! Results from storage adapters that update the domain vault graph.

use crate::{BackendObjectId, NodeId, NodeKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultEvent {
    Text {
        node: NodeId,
        text: String,
    },
    Bytes {
        node: NodeId,
        bytes: Vec<u8>,
    },
    Written {
        node: NodeId,
        revision: Option<String>,
    },
    Created {
        backend: BackendObjectId,
        parent: Option<BackendObjectId>,
        name: String,
        kind: NodeKind,
        revision: Option<String>,
    },
    Removed {
        node: NodeId,
    },
    Renamed {
        node: NodeId,
    },
    Error {
        node: Option<NodeId>,
        message: String,
    },
}
