//! Projects the local path-based vault into the shared node model.

use tft_domain::vault::{BackendObjectId, FileMeta, NodeKind, ScannedNode, VaultData};

use crate::Vault;

impl Vault {
    /// Fills `data` from this vault. Local object IDs are paths; a rename
    /// performed by TFT updates them, an external rename appears as a new node.
    pub fn apply_snapshot(&self, data: &mut VaultData) {
        let mut scanned = Vec::with_capacity(self.files().len() + self.folders().len());
        for folder in self.folders() {
            let (parent, name) = split(folder);
            scanned.push(ScannedNode {
                backend: BackendObjectId::Local(folder.clone()),
                parent: parent.map(|p| BackendObjectId::Local(p.to_string())),
                name: name.to_string(),
                kind: NodeKind::Folder,
                revision: None,
            });
        }
        for file in self.files() {
            let (parent, name) = split(&file.path);
            scanned.push(ScannedNode {
                backend: BackendObjectId::Local(file.path.clone()),
                parent: parent.map(|p| BackendObjectId::Local(p.to_string())),
                name: name.to_string(),
                kind: NodeKind::File(FileMeta {
                    kind: file.kind,
                    size: file.size,
                    mtime: file.mtime,
                    ctime: file.ctime,
                }),
                revision: None,
            });
        }
        data.apply_scan(scanned);
    }
}

fn split(path: &str) -> (Option<&str>, &str) {
    match path.rsplit_once('/') {
        Some((parent, name)) if !parent.is_empty() => (Some(parent), name),
        Some((_, name)) => (None, name),
        None => (None, path),
    }
}
