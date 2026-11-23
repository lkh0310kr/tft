//! Vault file metadata and the pure path plans every backend shares.

use crate::{validate_file_name, FileKind};
use serde::{Deserialize, Serialize};

/// One file in the vault. `path` is vault-relative with `/` separators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultFile {
    pub path: String,
    pub kind: FileKind,
    pub size: u64,
    /// Milliseconds since the Unix epoch.
    pub mtime: u64,
    /// Creation time in the same units.
    pub ctime: u64,
}

/// What a rescan found; paths are vault-relative.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChange {
    Added(String),
    Modified(String),
    Removed(String),
}

impl FileChange {
    pub fn path(&self) -> &str {
        match self {
            FileChange::Added(p) | FileChange::Modified(p) | FileChange::Removed(p) => p,
        }
    }
}

/// Checks every segment of a new vault-relative path.
pub fn validate_relative_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("File name cannot be empty.");
    }
    for segment in path.split('/') {
        validate_file_name(segment)?;
    }
    Ok(())
}

/// The files `rename(from, to)` moves, as `(old, new)`, without touching anything.
pub fn rename_plan(files: &[VaultFile], from: &str, to: &str) -> Vec<(String, String)> {
    let from = from.trim_matches('/');
    let to = to.trim_matches('/');
    let prefix = format!("{from}/");
    files
        .iter()
        .filter(|f| f.path == from || f.path.starts_with(&prefix))
        .map(|f| {
            let new = if f.path == from {
                to.to_string()
            } else {
                format!("{to}{}", &f.path[from.len()..])
            };
            (f.path.clone(), new)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> VaultFile {
        VaultFile {
            path: path.into(),
            kind: FileKind::from_path(path),
            size: 0,
            mtime: 0,
            ctime: 0,
        }
    }

    #[test]
    fn a_folder_rename_moves_every_file_below_it() {
        let files = vec![file("A.md"), file("Dir/A.md"), file("Dir/Sub/B.md")];
        assert_eq!(
            rename_plan(&files, "Dir", "Moved"),
            vec![
                ("Dir/A.md".into(), "Moved/A.md".into()),
                ("Dir/Sub/B.md".into(), "Moved/Sub/B.md".into())
            ]
        );
        assert_eq!(
            rename_plan(&files, "A.md", "B.md"),
            vec![("A.md".into(), "B.md".into())]
        );
    }

    #[test]
    fn paths_are_checked_one_segment_at_a_time() {
        assert!(validate_relative_path("Dir/Note.md").is_ok());
        assert!(validate_relative_path("").is_err());
        assert!(validate_relative_path("Dir/").is_err());
        assert!(validate_relative_path("../Note.md").is_err());
        assert!(validate_relative_path("Dir/Bad:Name.md").is_err());
    }
}
