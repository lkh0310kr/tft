//! Canonical vault node tree. Paths and lookup indexes are derived from nodes.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use tft_cache::{index_markdown, LinkResolver, MetadataCache};
use tft_formats::files::VaultFile;
use tft_formats::{validate_file_name, FileKind};

use crate::VaultEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BackendInstanceId(pub String);

/// Persistent identity of one storage object. Drive IDs survive rename/move;
/// a local path does not, so a local rename must update this value too.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BackendObjectId {
    Local(String),
    Drive(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileMeta {
    pub kind: FileKind,
    pub size: u64,
    pub mtime: u64,
    pub ctime: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Folder,
    File(FileMeta),
}

#[derive(Debug, Clone, PartialEq)]
pub struct VaultNode {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub name: String,
    pub kind: NodeKind,
    pub backend: BackendObjectId,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScannedNode {
    pub backend: BackendObjectId,
    pub parent: Option<BackendObjectId>,
    pub name: String,
    pub kind: NodeKind,
    pub revision: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameConflict {
    pub parent: NodeId,
    pub name_key: String,
    pub nodes: Vec<NodeId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LookupError {
    NotFound,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameError {
    NotFound,
    DestinationExists,
    IntoItself,
    InvalidName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvariantError {
    MissingRoot,
    RootHasParent,
    MissingParent(NodeId),
    Cycle(NodeId),
    FileHasChildren(NodeId),
    IndexMismatch,
}

#[derive(Debug, Default)]
struct Index {
    children: BTreeMap<NodeId, Vec<NodeId>>,
    by_backend: HashMap<BackendObjectId, NodeId>,
    names: BTreeMap<(NodeId, String), Vec<NodeId>>,
    paths: BTreeMap<NodeId, String>,
    by_path: BTreeMap<String, NodeId>,
    conflicts: Vec<NameConflict>,
}

#[derive(Debug)]
pub struct VaultData {
    pub backend_instance: BackendInstanceId,
    pub root: NodeId,
    nodes: BTreeMap<NodeId, VaultNode>,
    texts: HashMap<NodeId, Arc<String>>,
    next_id: u64,
    index: Index,
    pub cache: MetadataCache,
}

impl Default for VaultData {
    fn default() -> Self {
        let root = NodeId(1);
        let mut nodes = BTreeMap::new();
        nodes.insert(
            root,
            VaultNode {
                id: root,
                parent: None,
                name: String::new(),
                kind: NodeKind::Folder,
                backend: BackendObjectId::Local(String::new()),
                revision: None,
            },
        );
        let mut data = Self {
            backend_instance: BackendInstanceId("local".into()),
            root,
            nodes,
            texts: HashMap::new(),
            next_id: 2,
            index: Index::default(),
            cache: MetadataCache::new(
                LinkResolver::new(Vec::<String>::new()),
                Vec::<(String, tft_cache::CachedMetadata)>::new(),
            ),
        };
        data.rebuild_indexes();
        data
    }
}

impl VaultData {
    pub fn new(backend_instance: impl Into<String>) -> Self {
        Self {
            backend_instance: BackendInstanceId(backend_instance.into()),
            ..Self::default()
        }
    }

    /// Builds a graph from the old flat lists while callers migrate.
    pub fn from_flat(
        files: Vec<VaultFile>,
        folders: Vec<String>,
        texts: HashMap<String, Arc<String>>,
    ) -> Self {
        let mut data = Self::default();
        let mut folders = folders;
        folders.sort_by_key(|path| path.matches('/').count());
        for path in folders {
            let (parent, name) = split_path(&path);
            let parent = parent.map(|p| BackendObjectId::Local(p.to_string()));
            let name = name.to_string();
            data.upsert_object(
                BackendObjectId::Local(path),
                parent,
                name,
                NodeKind::Folder,
                None,
            );
        }
        for file in files {
            let (parent, name) = split_path(&file.path);
            let parent = parent.map(|p| BackendObjectId::Local(p.to_string()));
            let name = name.to_string();
            data.upsert_object(
                BackendObjectId::Local(file.path.clone()),
                parent,
                name,
                NodeKind::File(FileMeta {
                    kind: file.kind,
                    size: file.size,
                    mtime: file.mtime,
                    ctime: file.ctime,
                }),
                None,
            );
        }
        data.rebuild_indexes();
        for (path, text) in texts {
            if let Ok(id) = data.lookup(&path) {
                data.set_text(id, text.to_string());
            }
        }
        data
    }

    pub fn node(&self, id: NodeId) -> Option<&VaultNode> {
        self.nodes.get(&id)
    }

    /// Names the storage object a backend's root maps to; Drive needs it to
    /// resolve the parent of a new root-level object.
    pub fn set_root_backend(&mut self, backend: BackendObjectId) {
        if let Some(node) = self.nodes.get_mut(&self.root) {
            node.backend = backend;
        }
        self.rebuild_indexes();
    }

    pub fn nodes(&self) -> impl Iterator<Item = &VaultNode> {
        self.nodes.values()
    }

    pub fn children(&self, id: NodeId) -> &[NodeId] {
        self.index
            .children
            .get(&id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn backend(&self, id: NodeId) -> Option<&BackendObjectId> {
        self.nodes.get(&id).map(|n| &n.backend)
    }

    pub fn conflicts(&self) -> &[NameConflict] {
        &self.index.conflicts
    }

    /// Inserts or updates one backend object. Parent backend IDs are resolved
    /// through the current index; call `rebuild_indexes` after a batch.
    pub fn upsert_object(
        &mut self,
        backend: BackendObjectId,
        parent: Option<BackendObjectId>,
        name: String,
        kind: NodeKind,
        revision: Option<String>,
    ) -> NodeId {
        let parent_id = parent
            .as_ref()
            .and_then(|p| self.index.by_backend.get(p).copied())
            .unwrap_or(self.root);
        if let Some(&id) = self.index.by_backend.get(&backend) {
            if let Some(node) = self.nodes.get_mut(&id) {
                node.parent = Some(parent_id);
                node.name = name;
                node.kind = kind;
                node.revision = revision;
            }
            return id;
        }
        let id = NodeId(self.next_id);
        self.next_id += 1;
        self.nodes.insert(
            id,
            VaultNode {
                id,
                parent: Some(parent_id),
                name,
                kind,
                backend: backend.clone(),
                revision,
            },
        );
        self.index.by_backend.insert(backend, id);
        id
    }

    /// Applies a complete backend scan and removes objects no longer present.
    pub fn apply_scan(&mut self, scanned: Vec<ScannedNode>) {
        let mut seen = HashSet::new();
        for node in &scanned {
            seen.insert(node.backend.clone());
            let parent = node.parent.clone();
            self.upsert_object(
                node.backend.clone(),
                parent,
                node.name.clone(),
                node.kind.clone(),
                node.revision.clone(),
            );
        }
        let remove: Vec<NodeId> = self
            .nodes
            .values()
            .filter(|n| n.id != self.root && !seen.contains(&n.backend))
            .map(|n| n.id)
            .collect();
        for id in remove {
            self.remove(id);
        }
        self.rebuild_indexes();
    }

    /// Applies children discovered later (a lazy backend folder expansion)
    /// without removing objects that are still valid.
    pub fn apply_partial_scan(&mut self, scanned: Vec<ScannedNode>) {
        for node in scanned {
            self.upsert_object(
                node.backend,
                node.parent,
                node.name,
                node.kind,
                node.revision,
            );
        }
        self.rebuild_indexes();
    }

    pub fn remove(&mut self, id: NodeId) -> Vec<NodeId> {
        if id == self.root {
            return Vec::new();
        }
        let mut ids = Vec::new();
        self.collect_subtree(id, &mut ids);
        for id in &ids {
            self.nodes.remove(id);
            self.texts.remove(id);
        }
        self.rebuild_indexes();
        ids
    }

    /// Validates a name for a new child in the canonical Vault model. Hosts
    /// should repeat the collision check at the storage boundary to handle
    /// concurrent external changes.
    pub fn validate_new_child_name(&self, parent: NodeId, name: &str) -> Result<(), RenameError> {
        let Some(parent_node) = self.node(parent) else {
            return Err(RenameError::NotFound);
        };
        if !matches!(parent_node.kind, NodeKind::Folder) {
            return Err(RenameError::NotFound);
        }
        validate_file_name(name).map_err(|_| RenameError::InvalidName)?;
        let key = name_key(name);
        if self.children(parent).iter().any(|child| {
            self.node(*child)
                .is_some_and(|node| name_key(&node.name) == key)
        }) {
            return Err(RenameError::DestinationExists);
        }
        Ok(())
    }

    fn collect_subtree(&self, id: NodeId, out: &mut Vec<NodeId>) {
        out.push(id);
        for child in self.children(id) {
            self.collect_subtree(*child, out);
        }
    }

    pub fn rename(&mut self, id: NodeId, parent: NodeId, name: String) -> Result<(), RenameError> {
        if self.node(id).is_none() || self.node(parent).is_none() {
            return Err(RenameError::NotFound);
        }
        if parent == id || self.is_ancestor(id, parent) {
            return Err(RenameError::IntoItself);
        }
        validate_file_name(&name).map_err(|_| RenameError::InvalidName)?;
        let key = name_key(&name);
        if self.children(parent).iter().any(|child| {
            *child != id && self.node(*child).is_some_and(|n| name_key(&n.name) == key)
        }) {
            return Err(RenameError::DestinationExists);
        }
        if let Some(node) = self.nodes.get_mut(&id) {
            node.parent = Some(parent);
            node.name = name;
        }
        self.rebuild_indexes();
        Ok(())
    }

    fn is_ancestor(&self, ancestor: NodeId, mut node: NodeId) -> bool {
        while let Some(parent) = self.node(node).and_then(|n| n.parent) {
            if parent == ancestor {
                return true;
            }
            node = parent;
        }
        false
    }

    /// Rebuilds every derived index from `nodes`.
    pub fn rebuild_indexes(&mut self) {
        self.index = Index::default();
        self.index.paths.insert(self.root, String::new());
        for node in self.nodes.values() {
            if let Some(parent) = node.parent {
                self.index.children.entry(parent).or_default().push(node.id);
            }
            self.index.by_backend.insert(node.backend.clone(), node.id);
        }
        for children in self.index.children.values_mut() {
            children.sort_unstable();
        }
        let root = self.root;
        self.walk(root, "");
        self.rebuild_cache();
    }

    fn walk(&mut self, parent: NodeId, parent_path: &str) {
        let children = self.children(parent).to_vec();
        let mut by_name: BTreeMap<String, Vec<NodeId>> = BTreeMap::new();
        for id in &children {
            if let Some(node) = self.nodes.get(id) {
                by_name.entry(name_key(&node.name)).or_default().push(*id);
            }
        }
        for (key, ids) in &by_name {
            self.index.names.insert((parent, key.clone()), ids.clone());
            if ids.len() > 1 {
                self.index.conflicts.push(NameConflict {
                    parent,
                    name_key: key.clone(),
                    nodes: ids.clone(),
                });
                continue;
            }
            let Some(id) = ids.first() else {
                continue;
            };
            let node = self.nodes.get(id).expect("child");
            let path = if parent_path.is_empty() {
                node.name.clone()
            } else {
                format!("{parent_path}/{}", node.name)
            };
            self.index.by_path.insert(path_key(&path), *id);
            self.index.paths.insert(*id, path.clone());
            if matches!(node.kind, NodeKind::Folder) {
                self.walk(*id, &path);
            }
        }
    }

    pub fn path(&self, id: NodeId) -> Option<&str> {
        self.index.paths.get(&id).map(String::as_str)
    }

    /// Resolves a unique path by walking names. An ancestor conflict makes
    /// every path below it ambiguous, not missing.
    pub fn lookup(&self, path: &str) -> Result<NodeId, LookupError> {
        let trimmed = path.trim_matches('/');
        if trimmed.is_empty() {
            return Ok(self.root);
        }
        let mut current = self.root;
        for segment in trimmed.split('/') {
            let key = name_key(segment);
            let Some(nodes) = self.index.names.get(&(current, key)) else {
                return Err(LookupError::NotFound);
            };
            match nodes.as_slice() {
                [] => return Err(LookupError::NotFound),
                [only] => current = *only,
                _ => return Err(LookupError::Ambiguous),
            }
        }
        Ok(current)
    }

    pub fn file_ids(&self) -> Vec<(NodeId, VaultFile)> {
        self.nodes
            .values()
            .filter_map(|node| {
                let NodeKind::File(meta) = node.kind else {
                    return None;
                };
                let path = self.path(node.id)?;
                Some((
                    node.id,
                    VaultFile {
                        path: path.to_string(),
                        kind: meta.kind,
                        size: meta.size,
                        mtime: meta.mtime,
                        ctime: meta.ctime,
                    },
                ))
            })
            .collect()
    }

    pub fn files(&self) -> Vec<VaultFile> {
        self.file_ids().into_iter().map(|(_, file)| file).collect()
    }

    pub fn folders(&self) -> Vec<String> {
        self.nodes
            .values()
            .filter(|node| node.id != self.root && matches!(node.kind, NodeKind::Folder))
            .filter_map(|node| self.path(node.id).map(str::to_string))
            .collect()
    }

    pub fn text(&self, id: NodeId) -> Option<&Arc<String>> {
        self.texts.get(&id)
    }

    pub fn text_entries(&self) -> impl Iterator<Item = (NodeId, &Arc<String>)> {
        self.texts.iter().map(|(id, text)| (*id, text))
    }

    pub fn set_text(&mut self, id: NodeId, text: String) {
        let Some(path) = self.path(id).map(str::to_string) else {
            return;
        };
        let meta = index_markdown(&text);
        self.texts.insert(id, Arc::new(text));
        self.cache.insert_file(path, meta);
    }

    /// Forgets a text placeholder so a failed lazy read can be retried.
    pub fn clear_text(&mut self, id: NodeId) {
        self.texts.remove(&id);
    }

    /// Moves local backend tokens after a TFT rename, so the node that was at
    /// `old` keeps its NodeId at `new` and only its path index changes.
    pub fn rebase_local_prefix(&mut self, old: &str, new: &str) {
        let old = old.trim_matches('/');
        let new = new.trim_matches('/');
        if old.is_empty() || old == new {
            return;
        }
        for node in self.nodes.values_mut() {
            let path = match &node.backend {
                BackendObjectId::Local(path) => path.clone(),
                BackendObjectId::Drive(_) => continue,
            };
            let rest = if path == old {
                ""
            } else if let Some(rest) = path.strip_prefix(old).and_then(|p| p.strip_prefix('/')) {
                rest
            } else {
                continue;
            };
            node.backend = BackendObjectId::Local(if rest.is_empty() {
                new.to_string()
            } else {
                format!("{new}/{rest}")
            });
        }
        self.rebuild_indexes();
    }

    pub fn set_revision(&mut self, id: NodeId, revision: Option<String>) {
        if let Some(node) = self.nodes.get_mut(&id) {
            node.revision = revision;
        }
    }

    /// Folds a backend result into the graph. `false` means the event asked
    /// for a refresh the caller still has to perform.
    pub fn apply_event(&mut self, event: VaultEvent) -> bool {
        match event {
            VaultEvent::Text { node, text } => {
                self.set_text(node, text);
                true
            }
            VaultEvent::Bytes { .. } => true,
            VaultEvent::Written { node, revision } => {
                self.set_revision(node, revision);
                true
            }
            VaultEvent::Created {
                backend,
                parent,
                name,
                kind,
                revision,
            } => {
                self.upsert_object(backend, parent, name, kind, revision);
                self.rebuild_indexes();
                true
            }
            VaultEvent::Removed { node } => {
                self.remove(node);
                true
            }
            VaultEvent::Renamed { node } => {
                self.rebuild_indexes();
                self.node(node).is_some()
            }
            VaultEvent::Error { .. } => false,
        }
    }

    fn rebuild_cache(&mut self) {
        let files = self.files();
        let resolver = LinkResolver::new(files.iter().map(|f| f.path.as_str()));
        let notes: Vec<_> = files
            .iter()
            .filter_map(|f| {
                let id = self.lookup(&f.path).ok()?;
                let text = self.texts.get(&id)?;
                Some((f.path.clone(), index_markdown(text)))
            })
            .collect();
        self.cache = MetadataCache::new(resolver, notes);
    }

    pub fn validate(&self) -> Result<(), InvariantError> {
        let root = self
            .nodes
            .get(&self.root)
            .ok_or(InvariantError::MissingRoot)?;
        if root.parent.is_some() {
            return Err(InvariantError::RootHasParent);
        }
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        for node in self.nodes.values() {
            if let Some(parent) = node.parent {
                if !self.nodes.contains_key(&parent) {
                    return Err(InvariantError::MissingParent(node.id));
                }
                if matches!(
                    self.nodes.get(&parent).map(|n| &n.kind),
                    Some(NodeKind::File(_))
                ) {
                    return Err(InvariantError::FileHasChildren(parent));
                }
            }
            self.visit(node.id, &mut visiting, &mut visited)?;
        }
        if self.index.conflicts.len() != self.index.names.values().filter(|v| v.len() > 1).count() {
            return Err(InvariantError::IndexMismatch);
        }
        Ok(())
    }

    fn visit(
        &self,
        id: NodeId,
        visiting: &mut BTreeSet<NodeId>,
        visited: &mut BTreeSet<NodeId>,
    ) -> Result<(), InvariantError> {
        if visited.contains(&id) {
            return Ok(());
        }
        if !visiting.insert(id) {
            return Err(InvariantError::Cycle(id));
        }
        if let Some(parent) = self.nodes.get(&id).and_then(|n| n.parent) {
            self.visit(parent, visiting, visited)?;
        }
        visiting.remove(&id);
        visited.insert(id);
        Ok(())
    }
}

fn split_path(path: &str) -> (Option<&str>, &str) {
    match path.rsplit_once('/') {
        Some((parent, name)) if !parent.is_empty() => (Some(parent), name),
        Some((_, name)) => (None, name),
        None => (None, path),
    }
}

fn name_key(name: &str) -> String {
    name.to_lowercase()
}

fn path_key(path: &str) -> String {
    path.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(
        data: &mut VaultData,
        parent: Option<BackendObjectId>,
        name: &str,
        backend: &str,
    ) -> NodeId {
        data.upsert_object(
            BackendObjectId::Local(backend.into()),
            parent,
            name.into(),
            NodeKind::Folder,
            None,
        )
    }

    fn file(
        data: &mut VaultData,
        parent: Option<BackendObjectId>,
        name: &str,
        backend: &str,
    ) -> NodeId {
        data.upsert_object(
            BackendObjectId::Local(backend.into()),
            parent,
            name.into(),
            NodeKind::File(FileMeta {
                kind: FileKind::Markdown,
                size: 0,
                mtime: 0,
                ctime: 0,
            }),
            None,
        )
    }

    #[test]
    fn duplicate_names_are_ambiguous_including_descendants() {
        let mut data = VaultData::default();
        let a = folder(&mut data, None, "A", "a");
        let b = folder(&mut data, None, "A", "b");
        file(
            &mut data,
            Some(BackendObjectId::Local("a".into())),
            "X.md",
            "a-x",
        );
        file(
            &mut data,
            Some(BackendObjectId::Local("b".into())),
            "Y.md",
            "b-y",
        );
        data.rebuild_indexes();
        assert_eq!(data.lookup("A"), Err(LookupError::Ambiguous));
        assert_eq!(data.lookup("A/X.md"), Err(LookupError::Ambiguous));
        assert_eq!(data.path(a), None);
        assert_eq!(data.path(b), None);
        assert_eq!(data.conflicts().len(), 1);
    }

    #[test]
    fn same_name_in_different_parents_and_empty_folders_are_fine() {
        let mut data = VaultData::default();
        let a = folder(&mut data, None, "A", "a");
        let b = folder(&mut data, None, "B", "b");
        file(
            &mut data,
            Some(BackendObjectId::Local("a".into())),
            "Note.md",
            "a-note",
        );
        file(
            &mut data,
            Some(BackendObjectId::Local("b".into())),
            "Note.md",
            "b-note",
        );
        folder(
            &mut data,
            Some(BackendObjectId::Local("a".into())),
            "Empty",
            "empty",
        );
        data.rebuild_indexes();
        assert!(data.conflicts().is_empty());
        assert_eq!(data.path(a), Some("A"));
        assert_eq!(data.path(b), Some("B"));
        assert!(data.lookup("A/Empty").is_ok());
    }

    #[test]
    fn new_child_name_validation_is_shared_and_case_insensitive() {
        let mut data = VaultData::default();
        let notes = folder(&mut data, None, "Notes", "notes");
        let note = file(
            &mut data,
            Some(BackendObjectId::Local("notes".into())),
            "Welcome.md",
            "welcome",
        );
        data.rebuild_indexes();

        assert_eq!(data.validate_new_child_name(notes, "New.md"), Ok(()));
        assert_eq!(
            data.validate_new_child_name(notes, "welcome.MD"),
            Err(RenameError::DestinationExists)
        );
        assert_eq!(
            data.validate_new_child_name(note, "Child.md"),
            Err(RenameError::NotFound)
        );
        assert_eq!(
            data.validate_new_child_name(notes, "../escape.md"),
            Err(RenameError::InvalidName)
        );
        assert_eq!(
            data.validate_new_child_name(NodeId(u64::MAX), "New.md"),
            Err(RenameError::NotFound)
        );
    }

    #[test]
    fn folder_rename_moves_descendants_and_reuses_backend_ids() {
        let mut data = VaultData::default();
        let old = folder(&mut data, None, "Old", "folder");
        file(
            &mut data,
            Some(BackendObjectId::Local("folder".into())),
            "Child.md",
            "child",
        );
        data.rebuild_indexes();
        let child = data.lookup("Old/Child.md").unwrap();
        data.rename(old, data.root, "New".into()).unwrap();
        assert_eq!(data.path(child), Some("New/Child.md"));
        let again = folder(&mut data, None, "New", "folder");
        assert_eq!(again, old);
    }

    #[test]
    fn scan_keeps_backend_ids_and_removes_missing_objects() {
        let mut data = VaultData::default();
        data.apply_scan(vec![
            ScannedNode {
                backend: BackendObjectId::Drive("folder".into()),
                parent: None,
                name: "Notes".into(),
                kind: NodeKind::Folder,
                revision: None,
            },
            ScannedNode {
                backend: BackendObjectId::Drive("file".into()),
                parent: Some(BackendObjectId::Drive("folder".into())),
                name: "A.md".into(),
                kind: NodeKind::File(FileMeta {
                    kind: FileKind::Markdown,
                    size: 1,
                    mtime: 2,
                    ctime: 3,
                }),
                revision: None,
            },
        ]);
        let id = data.lookup("Notes/A.md").unwrap();
        data.apply_scan(vec![ScannedNode {
            backend: BackendObjectId::Drive("folder".into()),
            parent: None,
            name: "Notes".into(),
            kind: NodeKind::Folder,
            revision: None,
        }]);
        assert_eq!(data.lookup("Notes/A.md"), Err(LookupError::NotFound));
        assert!(data.node(id).is_none());
        assert!(data.children(data.root).len() == 1);
    }

    #[test]
    fn events_fold_back_into_the_graph() {
        let mut data = VaultData::default();
        let id = file(&mut data, None, "A.md", "a");
        data.rebuild_indexes();
        assert!(data.apply_event(VaultEvent::Text {
            node: id,
            text: "# A".into(),
        }));
        assert_eq!(data.text(id).map(|t| t.as_str()), Some("# A"));
        assert!(data.apply_event(VaultEvent::Written {
            node: id,
            revision: Some("r1".into()),
        }));
        assert_eq!(data.node(id).unwrap().revision.as_deref(), Some("r1"));

        let parent = folder(&mut data, None, "Notes", "folder");
        data.rebuild_indexes();
        assert!(data.apply_event(VaultEvent::Created {
            backend: BackendObjectId::Drive("new".into()),
            parent: Some(BackendObjectId::Local("folder".into())),
            name: "B.md".into(),
            kind: NodeKind::File(FileMeta {
                kind: FileKind::Markdown,
                size: 0,
                mtime: 0,
                ctime: 0,
            }),
            revision: None,
        }));
        assert!(data.lookup("Notes/B.md").is_ok());
        assert_eq!(data.children(parent).len(), 1);

        assert!(data.apply_event(VaultEvent::Removed { node: id }));
        assert_eq!(data.lookup("A.md"), Err(LookupError::NotFound));
    }
}
