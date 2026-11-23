//! A vault's file list without its filesystem: the plans every backend shares.

use std::collections::HashSet;
use tft_formats::files::{rename_plan, validate_relative_path, VaultFile};

#[derive(Debug, Clone, Default)]
pub struct VaultSnapshot {
    pub files: Vec<VaultFile>,
    pub folders: Vec<String>,
}

impl VaultSnapshot {
    pub fn new(files: Vec<VaultFile>, folders: Vec<String>) -> Self {
        Self { files, folders }
    }

    pub fn exists(&self, path: &str) -> bool {
        !path.is_empty()
            && (self.files.iter().any(|f| f.path.eq_ignore_ascii_case(path))
                || self.folders.iter().any(|f| f.eq_ignore_ascii_case(path)))
    }

    pub fn is_dir(&self, path: &str) -> bool {
        self.folders.iter().any(|f| f.eq_ignore_ascii_case(path))
    }

    pub fn files_under(&self, path: &str) -> Vec<&VaultFile> {
        let prefix = format!("{path}/");
        self.files
            .iter()
            .filter(|f| f.path == path || f.path.starts_with(&prefix))
            .collect()
    }

    pub fn rename_plan(&self, from: &str, to: &str) -> Vec<(String, String)> {
        rename_plan(&self.files, from, to)
    }

    /// The first free name among `stem`, `stem 1`, ... in `folder`.
    pub fn available_path(&self, folder: &str, stem: &str, ext: &str) -> String {
        let with_ext = |name: &str| {
            let name = if ext.is_empty() {
                name.to_string()
            } else {
                format!("{name}.{ext}")
            };
            tft_formats::paths::join(folder, &name)
        };
        let plain = with_ext(stem);
        if !self.exists(&plain) {
            return plain;
        }
        (1..)
            .map(|n| with_ext(&format!("{stem} {n}")))
            .find(|p| !self.exists(p))
            .expect("an unused number")
    }

    /// Whether a rename would go through, without touching anything.
    pub fn check_rename(&self, from: &str, to: &str) -> Result<(), RenameError> {
        let from = from.trim_matches('/');
        let to = to.trim_matches('/');
        validate_relative_path(to).map_err(RenameError::InvalidName)?;
        if !self.exists(from) {
            return Err(RenameError::NotFound);
        }
        if from == to {
            return Ok(());
        }
        let case_only = from.eq_ignore_ascii_case(to);
        if self.exists(to) && !case_only {
            return Err(RenameError::DestinationExists);
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(RenameError::IntoItself);
        }
        Ok(())
    }

    /// Every folder plus its ancestors, for a backend that only reports files.
    pub fn folders_with_ancestors(files: &[VaultFile]) -> Vec<String> {
        let mut folders = HashSet::new();
        for file in files {
            let mut end = 0;
            while let Some(i) = file.path[end..].find('/') {
                end += i;
                folders.insert(file.path[..end].to_string());
                end += 1;
            }
        }
        let mut folders: Vec<String> = folders.into_iter().collect();
        folders.sort();
        folders
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameError {
    NotFound,
    DestinationExists,
    IntoItself,
    InvalidName(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tft_formats::FileKind;

    fn file(path: &str) -> VaultFile {
        VaultFile {
            path: path.into(),
            kind: FileKind::from_path(path),
            size: 0,
            mtime: 0,
            ctime: 0,
        }
    }

    fn snapshot() -> VaultSnapshot {
        VaultSnapshot::new(
            vec![file("A.md"), file("Dir/B.md"), file("Dir/C.md")],
            vec!["Dir".into()],
        )
    }

    #[test]
    fn paths_and_renames_are_case_insensitive() {
        let v = snapshot();
        assert!(v.exists("DIR/b.md"));
        assert!(v.is_dir("dir"));
        assert_eq!(v.rename_plan("Dir", "Moved").len(), 2);
        assert_eq!(v.check_rename("Dir", "Moved"), Ok(()));
        assert_eq!(
            v.check_rename("Dir", "A.md"),
            Err(RenameError::DestinationExists)
        );
        assert_eq!(
            v.check_rename("Dir", "Dir/Sub"),
            Err(RenameError::IntoItself)
        );
    }

    #[test]
    fn available_paths_skip_taken_names() {
        let v = snapshot();
        assert_eq!(v.available_path("", "A", "md"), "A 1.md");
        assert_eq!(v.available_path("Dir", "B", "md"), "Dir/B 1.md");
        assert_eq!(v.available_path("New", "Note", "md"), "New/Note.md");
    }
}
