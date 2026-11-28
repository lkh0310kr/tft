//! Shared rows and availability for the Files tree context menu.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTreeMenuSection {
    Create,
    Open,
    Organize,
    Path,
    System,
    Danger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileTreeMenuAction {
    NewFile,
    NewFolder,
    NewCanvas,
    OpenInNewTab,
    OpenToRight,
    OpenInNewWindow,
    Duplicate,
    Move,
    SearchInFolder,
    CopyAbsolutePath,
    CopyRelativePath,
    OpenInDefaultApp,
    RevealInSystemExplorer,
    Rename,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTreeMenuItem {
    pub label: &'static str,
    pub section: FileTreeMenuSection,
    pub action: FileTreeMenuAction,
    pub enabled: bool,
}

pub fn separators_above(
    previous: Option<FileTreeMenuSection>,
    section: FileTreeMenuSection,
) -> usize {
    match previous {
        None => 0,
        Some(previous) if previous == section => 0,
        Some(_) if section == FileTreeMenuSection::Danger => 2,
        Some(_) => 1,
    }
}

/// The Files tree uses one ordered menu for files, folders, and blank space.
/// File creation acts in the parent folder, so it is available on every target.
pub fn file_tree_menu_items(
    is_file: bool,
    is_folder: bool,
    reveal_label: &'static str,
) -> Vec<FileTreeMenuItem> {
    use FileTreeMenuAction as A;
    use FileTreeMenuSection as S;
    let has_path = is_file || is_folder;
    [
        ("New File", S::Create, A::NewFile, true),
        ("New folder", S::Create, A::NewFolder, true),
        ("New canvas", S::Create, A::NewCanvas, true),
        ("Open in new tab", S::Open, A::OpenInNewTab, is_file),
        ("Open to the right", S::Open, A::OpenToRight, is_file),
        ("Open in new window", S::Open, A::OpenInNewWindow, is_file),
        ("Duplicate", S::Organize, A::Duplicate, has_path),
        ("Move to...", S::Organize, A::Move, has_path),
        ("Search in folder", S::Organize, A::SearchInFolder, has_path),
        ("Copy Path", S::Path, A::CopyAbsolutePath, has_path),
        ("Copy Relative Path", S::Path, A::CopyRelativePath, has_path),
        (
            "Open in default app",
            S::System,
            A::OpenInDefaultApp,
            is_file,
        ),
        (reveal_label, S::System, A::RevealInSystemExplorer, has_path),
        ("Rename...", S::Danger, A::Rename, has_path),
        ("Delete", S::Danger, A::Delete, has_path),
    ]
    .into_iter()
    .map(|(label, section, action, enabled)| FileTreeMenuItem {
        label,
        section,
        action,
        enabled,
    })
    .collect()
}
