//! Storage requests emitted by application use cases.

use tft_domain::NodeId;

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
