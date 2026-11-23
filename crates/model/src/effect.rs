//! I/O requests and results. The model never touches a filesystem or network.

use crate::{BackendObjectId, NodeId, NodeKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultEffect {
    ReadText {
        node: NodeId,
    },
    ReadBytes {
        node: NodeId,
    },
    WriteText {
        node: NodeId,
        text: String,
        expected_revision: Option<String>,
    },
    CreateText {
        parent: NodeId,
        name: String,
        text: String,
    },
    CreateFolder {
        parent: NodeId,
        name: String,
    },
    Rename {
        node: NodeId,
        parent: NodeId,
        name: String,
    },
    Delete {
        node: NodeId,
    },
}

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
