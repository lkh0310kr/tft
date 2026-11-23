//! The Files view model: ordering, collapse state and visible rows.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use tft_formats::files::VaultFile;
use tft_formats::FileKind;
use tft_search::natural_cmp;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    #[default]
    Alphabetical,
    AlphabeticalReverse,
    ByModifiedTime,
    ByModifiedTimeReverse,
    ByCreatedTime,
    ByCreatedTimeReverse,
}

impl SortOrder {
    /// Every order, in the order the menu lists them.
    pub const ALL: [SortOrder; 6] = [
        SortOrder::Alphabetical,
        SortOrder::AlphabeticalReverse,
        SortOrder::ByModifiedTime,
        SortOrder::ByModifiedTimeReverse,
        SortOrder::ByCreatedTime,
        SortOrder::ByCreatedTimeReverse,
    ];

    /// The string `workspace.json` records.
    pub fn as_str(self) -> &'static str {
        match self {
            SortOrder::Alphabetical => "alphabetical",
            SortOrder::AlphabeticalReverse => "alphabeticalReverse",
            SortOrder::ByModifiedTime => "byModifiedTime",
            SortOrder::ByModifiedTimeReverse => "byModifiedTimeReverse",
            SortOrder::ByCreatedTime => "byCreatedTime",
            SortOrder::ByCreatedTimeReverse => "byCreatedTimeReverse",
        }
    }

    pub fn parse(s: &str) -> Option<SortOrder> {
        SortOrder::ALL.into_iter().find(|o| o.as_str() == s)
    }

    /// The menu's label.
    pub fn label(self) -> &'static str {
        match self {
            SortOrder::Alphabetical => "File name (A to Z)",
            SortOrder::AlphabeticalReverse => "File name (Z to A)",
            SortOrder::ByModifiedTime => "Modified time (new to old)",
            SortOrder::ByModifiedTimeReverse => "Modified time (old to new)",
            SortOrder::ByCreatedTime => "Created time (new to old)",
            SortOrder::ByCreatedTimeReverse => "Created time (old to new)",
        }
    }

    /// How two files of one folder compare under this order. Names tie
    /// break every time sort, and the full file name breaks a tie between
    /// two display names, so the order of a rescan is stable.
    fn compare(self, a: &Entry, b: &Entry) -> Ordering {
        self.compare_parts(&a.name, a.mtime, a.ctime, &b.name, b.mtime, b.ctime)
    }

    fn compare_parts(
        self,
        a_name: &str,
        a_mtime: u64,
        a_ctime: u64,
        b_name: &str,
        b_mtime: u64,
        b_ctime: u64,
    ) -> Ordering {
        let by_name = || {
            natural_cmp(display_name(a_name), display_name(b_name))
                .then_with(|| natural_cmp(a_name, b_name))
        };
        match self {
            SortOrder::Alphabetical => by_name(),
            SortOrder::AlphabeticalReverse => {
                natural_cmp(display_name(b_name), display_name(a_name))
                    .then_with(|| natural_cmp(a_name, b_name))
            }
            SortOrder::ByModifiedTime => b_mtime.cmp(&a_mtime).then_with(by_name),
            SortOrder::ByModifiedTimeReverse => a_mtime.cmp(&b_mtime).then_with(by_name),
            SortOrder::ByCreatedTime => b_ctime.cmp(&a_ctime).then_with(by_name),
            SortOrder::ByCreatedTimeReverse => a_ctime.cmp(&b_ctime).then_with(by_name),
        }
    }
}

/// One line of the tree with every folder expanded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Stable node identity when the row came from `VaultData`.
    pub node: Option<tft_model::NodeId>,
    /// Collapse key; the path for the legacy path tree, `#id` for a conflict.
    pub key: String,
    /// Vault-relative path; empty for a conflicted node.
    pub path: String,
    pub name: String,
    pub tag: Option<String>,
    pub depth: usize,
    pub is_folder: bool,
    pub kind: FileKind,
    pub conflict: bool,
}

/// The folder a drop over a row lands in.
pub fn drop_folder(row_path: &str, row_is_folder: bool) -> String {
    if row_is_folder {
        row_path.to_string()
    } else {
        row_path
            .rsplit_once('/')
            .map(|(dir, _)| dir.to_string())
            .unwrap_or_default()
    }
}

/// The ghost's action line for a target folder: `Move into ?쏡est??, the
/// vault's own name standing for the root (`s03`).
pub fn move_into_label(folder: &str, vault_name: &str) -> String {
    let name = if folder.is_empty() {
        vault_name
    } else {
        folder.rsplit('/').next().unwrap_or(folder)
    };
    format!("Move into \u{201c}{name}\u{201d}")
}

/// Whether dropping `path` in `folder` moves anything: not into the folder
/// it is already in, and a folder not into itself or anything inside it,
/// which Obsidian refused by leaving the tree byte-identical (2026-10-25).
pub fn drop_moves(path: &str, path_is_folder: bool, folder: &str) -> bool {
    let parent = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    if parent == folder {
        return false;
    }
    !(path_is_folder && (folder == path || folder.starts_with(&format!("{path}/"))))
}

/// A file as its folder holds it: what the sort orders need of it.
#[derive(Debug, Clone)]
struct Entry {
    name: String,
    kind: FileKind,
    mtime: u64,
    ctime: u64,
}

#[derive(Debug, Default)]
struct Folder {
    folders: BTreeMap<String, Folder>,
    files: Vec<Entry>,
}

/// One node of the [`VaultData`] tree, kept so a sort can re-flatten without
/// the vault itself.
#[derive(Debug, Clone)]
struct VaultEntry {
    id: tft_model::NodeId,
    name: String,
    path: String,
    key: String,
    kind: FileKind,
    mtime: u64,
    ctime: u64,
    is_folder: bool,
    conflict: bool,
    children: Vec<VaultEntry>,
}

/// Folder tree with per-folder collapse state.
#[derive(Debug, Default)]
pub struct FileTree {
    root: Folder,
    vault: Option<Vec<VaultEntry>>,
    sort: SortOrder,
    all: Vec<Row>,
    collapsed: HashSet<String>,
    visible: Vec<usize>,
}

impl FileTree {
    /// Builds the tree. Files of unknown kind are hidden unless
    /// `show_unsupported` (Obsidian's "Detect all file extensions");
    /// `folders` adds the empty folders the files alone would not show.
    pub fn new<'a>(
        files: impl IntoIterator<Item = &'a VaultFile>,
        folders: impl IntoIterator<Item = &'a String>,
        show_unsupported: bool,
    ) -> FileTree {
        let mut root = Folder::default();
        for path in folders {
            let mut folder = &mut root;
            for part in path.split('/').filter(|p| !p.is_empty()) {
                folder = folder.folders.entry(part.to_string()).or_default();
            }
        }
        for file in files {
            if file.kind == FileKind::Other && !show_unsupported {
                continue;
            }
            let mut parts: Vec<&str> = file.path.split('/').collect();
            let name = parts.pop().unwrap_or_default().to_string();
            let mut folder = &mut root;
            for part in parts {
                folder = folder.folders.entry(part.to_string()).or_default();
            }
            folder.files.push(Entry {
                name,
                kind: file.kind,
                mtime: file.mtime,
                ctime: file.ctime,
            });
        }
        let mut all = Vec::new();
        flatten(&root, "", 0, SortOrder::Alphabetical, &mut all);
        let collapsed = all
            .iter()
            .filter(|r| r.is_folder)
            .map(|r| r.key.clone())
            .collect();
        let mut tree = FileTree {
            root,
            vault: None,
            sort: SortOrder::Alphabetical,
            all,
            collapsed,
            visible: Vec::new(),
        };
        tree.recompute();
        tree
    }

    pub fn from_vault(data: &tft_model::VaultData) -> FileTree {
        Self::from_vault_sorted(data, SortOrder::Alphabetical)
    }

    pub fn from_vault_sorted(data: &tft_model::VaultData, sort: SortOrder) -> FileTree {
        let vault = data
            .children(data.root)
            .iter()
            .filter_map(|id| vault_entry(data, *id))
            .collect::<Vec<_>>();
        let mut all = Vec::new();
        flatten_vault(&vault, 0, sort, &mut all);
        let collapsed = all
            .iter()
            .filter(|r| r.is_folder)
            .map(|r| r.key.clone())
            .collect();
        let mut tree = FileTree {
            root: Folder::default(),
            vault: Some(vault),
            sort,
            all,
            collapsed,
            visible: Vec::new(),
        };
        tree.recompute();
        tree
    }

    pub fn sort(&self) -> SortOrder {
        self.sort
    }

    /// Re-lists every folder's files in `sort`; folders and their collapse
    /// state stay as they were.
    pub fn set_sort(&mut self, sort: SortOrder) {
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        self.all.clear();
        match &self.vault {
            Some(vault) => flatten_vault(vault, 0, sort, &mut self.all),
            None => flatten(&self.root, "", 0, sort, &mut self.all),
        }
        self.recompute();
    }

    fn recompute(&mut self) {
        self.visible.clear();
        let mut hide_below: Option<usize> = None;
        for (ix, row) in self.all.iter().enumerate() {
            if let Some(depth) = hide_below {
                if row.depth > depth {
                    continue;
                }
                hide_below = None;
            }
            self.visible.push(ix);
            if row.is_folder && self.collapsed.contains(&row.key) {
                hide_below = Some(row.depth);
            }
        }
    }

    /// Rows currently shown, in order.
    pub fn rows(&self) -> impl ExactSizeIterator<Item = &Row> {
        self.visible.iter().map(|&ix| &self.all[ix])
    }

    pub fn row(&self, visible_ix: usize) -> Option<&Row> {
        self.visible.get(visible_ix).map(|&ix| &self.all[ix])
    }

    pub fn len(&self) -> usize {
        self.visible.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// Folders currently expanded, to carry over to a rebuilt tree.
    pub fn expanded_folders(&self) -> HashSet<String> {
        self.all
            .iter()
            .filter(|r| r.is_folder && !self.collapsed.contains(&r.key))
            .map(|r| r.key.clone())
            .collect()
    }

    /// Expands the given folders (unknown ones are ignored).
    pub fn expand_folders(&mut self, folders: &HashSet<String>) {
        self.collapsed.retain(|f| !folders.contains(f));
        self.recompute();
    }

    pub fn is_collapsed(&self, folder: &str) -> bool {
        self.collapsed.contains(folder)
    }

    pub fn is_collapsed_key(&self, key: &str) -> bool {
        self.collapsed.contains(key)
    }

    pub fn toggle(&mut self, folder: &str) {
        self.toggle_key(folder);
    }

    pub fn toggle_key(&mut self, key: &str) {
        if !self.collapsed.remove(key) {
            self.collapsed.insert(key.to_string());
        }
        self.recompute();
    }

    pub fn collapse_all(&mut self) {
        self.collapsed = self
            .all
            .iter()
            .filter(|r| r.is_folder)
            .map(|r| r.key.clone())
            .collect();
        self.recompute();
    }

    pub fn expand_all(&mut self) {
        self.collapsed.clear();
        self.recompute();
    }

    /// Whether any folder is open: the header's last button is "Collapse
    /// all" then and "Expand all" otherwise, as Obsidian's swaps.
    pub fn any_expanded(&self) -> bool {
        self.all
            .iter()
            .any(|r| r.is_folder && !self.collapsed.contains(&r.key))
    }

    /// Expands every ancestor of `path` and returns its visible row index.
    pub fn reveal(&mut self, path: &str) -> Option<usize> {
        let mut prefix = String::new();
        for part in path.split('/') {
            if !prefix.is_empty() {
                self.collapsed.remove(&prefix);
                prefix.push('/');
            }
            prefix.push_str(part);
        }
        self.recompute();
        self.visible_index(path)
    }

    pub fn visible_index(&self, path: &str) -> Option<usize> {
        self.visible
            .iter()
            .position(|&ix| self.all[ix].path == path)
    }
}

fn flatten(folder: &Folder, prefix: &str, depth: usize, sort: SortOrder, out: &mut Vec<Row>) {
    let mut names: Vec<&String> = folder.folders.keys().collect();
    names.sort_by(|a, b| natural_cmp(a, b));
    for name in names {
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        out.push(Row {
            node: None,
            key: path.clone(),
            path: path.clone(),
            name: name.clone(),
            depth,
            tag: None,
            is_folder: true,
            kind: FileKind::Other,
            conflict: false,
        });
        flatten(&folder.folders[name], &path, depth + 1, sort, out);
    }
    let mut files: Vec<&Entry> = folder.files.iter().collect();
    files.sort_by(|a, b| sort.compare(a, b));
    for entry in files {
        let name = &entry.name;
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        out.push(Row {
            node: None,
            key: path.clone(),
            path,
            name: stem(name).to_string(),
            tag: file_tag(name),
            depth,
            is_folder: false,
            kind: entry.kind,
            conflict: false,
        });
    }
}

fn vault_entry(data: &tft_model::VaultData, id: tft_model::NodeId) -> Option<VaultEntry> {
    let node = data.node(id)?;
    let path = data.path(id).unwrap_or("").to_string();
    let key = if path.is_empty() {
        format!("#{}", id.0)
    } else {
        path.clone()
    };
    let (kind, mtime, ctime, is_folder) = match node.kind {
        tft_model::NodeKind::Folder => (FileKind::Other, 0, 0, true),
        tft_model::NodeKind::File(meta) => (meta.kind, meta.mtime, meta.ctime, false),
    };
    let children = data
        .children(id)
        .iter()
        .filter_map(|child| vault_entry(data, *child))
        .collect();
    Some(VaultEntry {
        id,
        name: node.name.clone(),
        path,
        key,
        kind,
        mtime,
        ctime,
        is_folder,
        conflict: data.path(id).is_none(),
        children,
    })
}

fn flatten_vault(entries: &[VaultEntry], depth: usize, sort: SortOrder, out: &mut Vec<Row>) {
    let mut folders: Vec<&VaultEntry> = entries.iter().filter(|e| e.is_folder).collect();
    folders.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    for entry in folders {
        out.push(Row {
            node: Some(entry.id),
            key: entry.key.clone(),
            path: entry.path.clone(),
            name: entry.name.clone(),
            tag: None,
            depth,
            is_folder: true,
            kind: FileKind::Other,
            conflict: entry.conflict,
        });
        flatten_vault(&entry.children, depth + 1, sort, out);
    }
    let mut files: Vec<&VaultEntry> = entries.iter().filter(|e| !e.is_folder).collect();
    files.sort_by(|a, b| sort.compare_parts(&a.name, a.mtime, a.ctime, &b.name, b.mtime, b.ctime));
    for entry in files {
        out.push(Row {
            node: Some(entry.id),
            key: entry.key.clone(),
            path: entry.path.clone(),
            name: stem(&entry.name).to_string(),
            tag: file_tag(&entry.name),
            depth,
            is_folder: false,
            kind: entry.kind,
            conflict: entry.conflict,
        });
    }
}

/// The name a [`FileKind`] goes by outside the app: in the control socket's
/// state dump, and anywhere else a kind has to survive as text.
pub fn kind_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Markdown => "markdown",
        FileKind::Canvas => "canvas",
        FileKind::Image => "image",
        FileKind::Audio => "audio",
        FileKind::Video => "video",
        FileKind::Pdf => "pdf",
        FileKind::Other => "other",
    }
}

/// A file's name without its extension, for the explorer's rows: Obsidian
/// shows every file bare and tags the ones that are not Markdown.
fn stem(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => stem,
        _ => file_name,
    }
}

/// The row's tag for a file that is not Markdown: its extension in upper
/// case (`CANVAS`, `PNG`); none for a note or a file without one.
fn file_tag(file_name: &str) -> Option<String> {
    match file_name.rsplit_once('.') {
        Some((stem, ext))
            if !stem.is_empty() && !ext.is_empty() && !ext.eq_ignore_ascii_case("md") =>
        {
            Some(ext.to_ascii_uppercase())
        }
        _ => None,
    }
}

/// Markdown files show without their extension, everything else with it.
pub fn display_name(file_name: &str) -> &str {
    match file_name.rsplit_once('.') {
        Some((stem, ext)) if ext.eq_ignore_ascii_case("md") && !stem.is_empty() => stem,
        _ => file_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a drop lands and what the ghost says, from the 2026-11-07
    /// holds: a folder row is its own target, a file row its folder's, a
    /// root file the vault's.
    #[test]
    fn a_drop_lands_in_the_folder_under_the_pointer() {
        assert_eq!(drop_folder("Dest", true), "Dest");
        assert_eq!(drop_folder("Inbox/Item.md", false), "Inbox");
        assert_eq!(drop_folder("Other.md", false), "");
        assert_eq!(
            move_into_label("Inbox/Deep", "vault"),
            "Move into \u{201c}Deep\u{201d}"
        );
        assert_eq!(
            move_into_label("", "My vault"),
            "Move into \u{201c}My vault\u{201d}"
        );
    }

    /// The moves Obsidian made and refused on 2026-10-25: into another
    /// folder yes; into the folder it is in, and a folder into itself or
    /// below itself, nothing.
    #[test]
    fn a_drop_moves_only_somewhere_else() {
        assert!(drop_moves("Root.md", false, "Dest"));
        assert!(drop_moves("Inbox/Item.md", false, ""));
        assert!(drop_moves("Deep", true, "Dest"));
        assert!(!drop_moves("Inbox/Item.md", false, "Inbox"));
        assert!(!drop_moves("Root.md", false, ""));
        assert!(!drop_moves("Deep", true, "Deep"));
        assert!(!drop_moves("Deep", true, "Deep/Sub"));
        // A sibling whose name starts the same is not inside.
        assert!(drop_moves("Deep", true, "Deeper"));
    }

    fn files(paths: &[&str]) -> Vec<VaultFile> {
        paths
            .iter()
            .map(|p| VaultFile {
                path: p.to_string(),
                kind: FileKind::from_path(p),
                size: 0,
                mtime: 0,
                ctime: 0,
            })
            .collect()
    }

    /// The orders Obsidian showed on 2026-10-21 for a vault whose files
    /// were created in one order and modified in another (the
    /// `explorer-header` probe): folders stay in name order under every
    /// sort, and only the files of each folder move.
    #[test]
    fn the_six_sort_orders_move_files_but_not_folders() {
        let created = [
            "Alpha.md",
            "beta.md",
            "Gamma.md",
            "n2.md",
            "n10.md",
            "Projects/Plan.md",
            "Projects/Deep/Inner.md",
            "Archive/old.md",
            "Projects/Budget.md",
            "cover.png",
        ];
        let modified = [
            "n10.md",
            "Archive/old.md",
            "Alpha.md",
            "Projects/Budget.md",
            "cover.png",
            "Gamma.md",
            "Projects/Deep/Inner.md",
            "beta.md",
            "Projects/Plan.md",
            "n2.md",
        ];
        let files: Vec<VaultFile> = created
            .iter()
            .enumerate()
            .map(|(ix, p)| VaultFile {
                path: p.to_string(),
                kind: FileKind::from_path(p),
                size: 0,
                mtime: 1 + modified.iter().position(|m| m == p).unwrap() as u64,
                ctime: 1 + ix as u64,
            })
            .collect();
        let empty = vec!["Empty".to_string()];
        let mut tree = FileTree::new(&files, &empty, false);
        tree.expand_all();
        let names = |tree: &FileTree| -> Vec<String> {
            tree.rows()
                .map(|r| {
                    if r.is_folder {
                        format!("{}/", r.name)
                    } else {
                        r.name.clone()
                    }
                })
                .collect()
        };
        let observed: [(SortOrder, &[&str]); 6] = [
            (
                SortOrder::Alphabetical,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Budget",
                    "Plan",
                    "Alpha",
                    "beta",
                    "cover",
                    "Gamma",
                    "n2",
                    "n10",
                ],
            ),
            (
                SortOrder::AlphabeticalReverse,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Plan",
                    "Budget",
                    "n10",
                    "n2",
                    "Gamma",
                    "cover",
                    "beta",
                    "Alpha",
                ],
            ),
            (
                SortOrder::ByModifiedTime,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Plan",
                    "Budget",
                    "n2",
                    "beta",
                    "Gamma",
                    "cover",
                    "Alpha",
                    "n10",
                ],
            ),
            (
                SortOrder::ByModifiedTimeReverse,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Budget",
                    "Plan",
                    "n10",
                    "Alpha",
                    "cover",
                    "Gamma",
                    "beta",
                    "n2",
                ],
            ),
            (
                SortOrder::ByCreatedTime,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Budget",
                    "Plan",
                    "cover",
                    "n10",
                    "n2",
                    "Gamma",
                    "beta",
                    "Alpha",
                ],
            ),
            (
                SortOrder::ByCreatedTimeReverse,
                &[
                    "Archive/",
                    "old",
                    "Empty/",
                    "Projects/",
                    "Deep/",
                    "Inner",
                    "Plan",
                    "Budget",
                    "Alpha",
                    "beta",
                    "Gamma",
                    "n2",
                    "n10",
                    "cover",
                ],
            ),
        ];
        for (order, rows) in observed {
            tree.set_sort(order);
            assert_eq!(names(&tree), rows, "{}", order.as_str());
            assert_eq!(SortOrder::parse(order.as_str()), Some(order));
        }
        // A collapsed folder stays collapsed across a re-sort.
        tree.toggle("Projects");
        tree.set_sort(SortOrder::Alphabetical);
        assert!(tree.is_collapsed("Projects"));
        assert!(tree.any_expanded(), "Archive is still open");
        tree.collapse_all();
        assert!(!tree.any_expanded());
    }

    #[test]
    fn folders_first_then_files_case_insensitively() {
        let files = files(&[
            "zeta.md",
            "Alpha.md",
            "b/inner.md",
            "a/x.png",
            "a/deep/y.md",
            "notes.zip",
        ]);
        let tree = FileTree::new(&files, &[], false);
        let names: Vec<(usize, &str)> = tree.rows().map(|r| (r.depth, r.name.as_str())).collect();
        assert_eq!(names, vec![(0, "a"), (0, "b"), (0, "Alpha"), (0, "zeta")]);
        let empty = vec!["Empty".to_string(), "a/void".to_string()];
        let with_empty = FileTree::new(&files, &empty, false);
        let names: Vec<&str> = with_empty.rows().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "Empty", "Alpha", "zeta"]);
        let shown = FileTree::new(&files, &[], true);
        assert_eq!(
            shown.len(),
            5,
            "notes.zip appears with unsupported files on"
        );
    }

    /// Obsidian collates names numerically, so `n2` comes before `n10`; the
    /// search pane already does and the two panes have to agree.
    #[test]
    fn names_sort_numerically_within_each_group() {
        let files = files(&[
            "n10.md",
            "n2.md",
            "n1.md",
            "n20.md",
            "n3.md",
            "f10/a.md",
            "f2/a.md",
            "Img 10.png",
            "Img 2.png",
        ]);
        let tree = FileTree::new(&files, &[], false);
        let names: Vec<&str> = tree.rows().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["f2", "f10", "Img 2", "Img 10", "n1", "n2", "n3", "n10", "n20"],
            "folders stay ahead of files, and both groups count"
        );
    }

    #[test]
    fn from_vault_keeps_empty_folders_and_marks_conflicts() {
        use tft_model::vault::{BackendObjectId, FileMeta, NodeKind, VaultData};
        let mut data = VaultData::new("test");
        data.upsert_object(
            BackendObjectId::Drive("empty".into()),
            None,
            "Empty".into(),
            NodeKind::Folder,
            None,
        );
        for (id, path) in [("a1", "A.md"), ("a2", "A.md"), ("b", "b.md")] {
            data.upsert_object(
                BackendObjectId::Drive(id.into()),
                None,
                path.into(),
                NodeKind::File(FileMeta {
                    kind: FileKind::Markdown,
                    size: 0,
                    mtime: 0,
                    ctime: 0,
                }),
                None,
            );
        }
        data.rebuild_indexes();
        let mut tree = FileTree::from_vault(&data);
        tree.expand_all();
        let rows: Vec<&Row> = tree.rows().collect();
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["Empty", "A", "A", "b"]
        );
        assert_eq!(rows[0].path, "Empty");
        assert!(rows[1].conflict && rows[2].conflict);
        assert!(rows[1].path.is_empty() && rows[2].path.is_empty());
        assert_ne!(rows[1].key, rows[2].key, "each conflict row toggles alone");
        assert_eq!(rows[3].path, "b.md");
        assert!(!rows[3].conflict);
        tree.set_sort(SortOrder::AlphabeticalReverse);
        assert_eq!(tree.len(), 4, "a sort keeps the vault tree");
    }

    #[test]
    fn toggle_and_reveal_expand_ancestors() {
        let files = files(&["a/deep/y.md", "a/x.png", "b/inner.md"]);
        let mut tree = FileTree::new(&files, &[], false);
        assert_eq!(tree.len(), 2);
        tree.toggle("a");
        let names: Vec<&str> = tree.rows().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "deep", "x", "b"]);
        tree.toggle("a");
        assert_eq!(tree.reveal("a/deep/y.md"), Some(2));
        let names: Vec<&str> = tree.rows().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["a", "deep", "y", "x", "b"]);
        assert!(tree.rows().nth(2).unwrap().path == "a/deep/y.md");
        tree.collapse_all();
        assert_eq!(tree.len(), 2);
        tree.expand_all();
        assert_eq!(tree.len(), 6);
        let expanded = tree.expanded_folders();
        let mut rebuilt = FileTree::new(&files, &[], false);
        rebuilt.expand_folders(&expanded);
        assert_eq!(rebuilt.len(), 6);
    }
}
