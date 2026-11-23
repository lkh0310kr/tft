//! Local file operations: create, rename/move, copy and trash. Each changes
//! the disk first, then the cached file list. Link rewriting lives in `cache`.
//! Obsidian's naming and collision rules are covered by the tests below.

use crate::{FileChange, Vault};
use std::path::{Path, PathBuf};

/// `trashOption` in `app.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrashOption {
    /// The operating system's trash (Obsidian's default).
    #[default]
    System,
    /// The vault's own `.trash` folder.
    Local,
    /// Delete permanently.
    None,
}

impl TrashOption {
    pub fn from_config(value: Option<&str>) -> TrashOption {
        match value {
            Some("local") => TrashOption::Local,
            Some("none") => TrashOption::None,
            _ => TrashOption::System,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error("File already exists.")]
    Exists,
    #[error("Destination file already exists!")]
    DestinationExists,
    #[error("{0}")]
    InvalidName(&'static str),
    #[error("{0} does not exist")]
    NotFound(String),
    #[error("{action} {path}: {source}")]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// A path the vault refuses to write: outside the folder, or through a
    /// symbolic link.
    #[error("{0}")]
    Refused(#[from] crate::VaultError),
}

impl OpError {
    /// The sentence a dialog shows. Everything but an I/O failure already
    /// reads as one; an I/O failure's `Display` carries the absolute path
    /// and the errno, which belong in a log and not in a box the user
    /// typed a file name into.
    pub fn user_message(&self) -> String {
        match self {
            OpError::Io {
                action,
                path,
                source,
            } => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("that file");
                format!("Could not {action} {name}: {}.", why(source))
            }
            other => other.to_string(),
        }
    }
}

/// Why an I/O call failed, in words, with no errno in them.
fn why(err: &std::io::Error) -> &'static str {
    match err.kind() {
        std::io::ErrorKind::PermissionDenied => "permission was denied",
        std::io::ErrorKind::NotFound => "it is no longer there",
        std::io::ErrorKind::AlreadyExists => "something is already there",
        std::io::ErrorKind::InvalidInput => "the name is not valid",
        _ => "the file system refused",
    }
}

fn io<'p>(action: &'static str, path: &'p Path) -> impl FnOnce(std::io::Error) -> OpError + 'p {
    move |source| OpError::Io {
        action,
        path: path.to_path_buf(),
        source,
    }
}

pub use tft_formats::paths::{attachment_folder, attachment_path, folder_of, join};

/// Checks every segment of a new path the way Obsidian's rename does.
fn validate_path(path: &str) -> Result<(), OpError> {
    tft_formats::files::validate_relative_path(path).map_err(OpError::InvalidName)
}

impl Vault {
    /// Whether anything exists at this vault path on disk.
    pub fn exists(&self, rel: &str) -> bool {
        !rel.is_empty() && self.absolute(rel).exists()
    }

    /// The first free name among `stem`, `stem 1`, `stem 2`, ... in `folder`,
    /// with `.ext` appended when `ext` is not empty (Obsidian's
    /// `getAvailablePath`).
    pub fn available_path(&self, folder: &str, stem: &str, ext: &str) -> String {
        self.numbered_path(folder, stem, ext, 1)
    }

    fn numbered_path(&self, folder: &str, stem: &str, ext: &str, first: usize) -> String {
        let with_ext = |name: &str| {
            if ext.is_empty() {
                join(folder, name)
            } else {
                join(folder, &format!("{name}.{ext}"))
            }
        };
        let plain = with_ext(stem);
        if !self.exists(&plain) {
            return plain;
        }
        let mut n = first;
        loop {
            let candidate = with_ext(&format!("{stem} {n}"));
            if !self.exists(&candidate) {
                return candidate;
            }
            n += 1;
        }
    }

    /// The folder a new note goes into: `newFileLocation` is `root`,
    /// `current` (the open note's folder) or `folder` (`newFileFolderPath`,
    /// falling back to the root when that folder is missing).
    pub fn new_note_folder(
        &self,
        location: Option<&str>,
        folder_path: Option<&str>,
        current: Option<&str>,
    ) -> String {
        match location {
            Some("current") => current.map(folder_of).unwrap_or("").to_string(),
            Some("folder") => {
                let folder = folder_path
                    .unwrap_or("")
                    .trim_matches('/')
                    .replace('\\', "/");
                if !folder.is_empty() && self.absolute(&folder).is_dir() {
                    folder
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        }
    }

    /// The path a note created for the link text `name` (without `.md`) gets:
    /// a name with a folder in it is taken from the vault root, anything
    /// else goes into the new-note folder.
    pub fn new_note_path(&self, name: &str, new_note_folder: &str) -> String {
        let name = name.trim().trim_matches('/');
        let name = name.strip_suffix(".md").unwrap_or(name);
        if name.contains('/') {
            format!("{name}.md")
        } else {
            join(new_note_folder, &format!("{name}.md"))
        }
    }

    /// Creates a markdown note (and its folders) with `text`. Fails when the
    /// path exists.
    pub fn create_note(&mut self, path: &str, text: &str) -> Result<Vec<FileChange>, OpError> {
        validate_path(path)?;
        if self.exists(path) {
            return Err(OpError::Exists);
        }
        let abs = self.write_path(path)?;
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(io("create folder", parent))?;
        }
        crate::write_no_follow(&abs, text.as_bytes()).map_err(io("create", &abs))?;
        Ok(self.refresh_path(&abs))
    }

    /// Creates an empty folder (and its parents).
    pub fn create_folder(&mut self, path: &str) -> Result<(), OpError> {
        validate_path(path)?;
        if self.exists(path) {
            return Err(OpError::Exists);
        }
        let abs = self.write_path(path)?;
        std::fs::create_dir_all(&abs).map_err(io("create folder", &abs))?;
        self.refresh_path(&abs);
        Ok(())
    }

    /// Copies a file or folder next to itself and returns the copy's vault
    /// path with the changes the index needs. The copy takes the first free
    /// name [`Vault::available_path`] gives the original's: `Name 1.ext`,
    /// `Name 2.ext`, ... for a file (the number goes before the extension)
    /// and `Name 1`, `Name 2`, ... for a folder, whose name is never split
    /// on a dot.
    ///
    /// A folder copy takes what the scanner would index -- every regular
    /// file and folder under it, dot-prefixed entries and links left out, as
    /// in [`Vault::rescan`] -- and copies each file's bytes verbatim. No link
    /// is rewritten: what Obsidian does to the links of the notes in a copied
    /// folder is unobserved, and a byte-identical copy loses nothing. The
    /// copy is a sibling of the original, so it can never land inside the
    /// folder it is copying.
    pub fn duplicate(&mut self, path: &str) -> Result<(String, Vec<FileChange>), OpError> {
        let path = path.trim_matches('/');
        let from_abs = self.entry_path(path)?;
        let meta = std::fs::metadata(&from_abs).map_err(|_| OpError::NotFound(path.to_string()))?;
        let (folder, name) = match path.rsplit_once('/') {
            Some((d, n)) => (d, n),
            None => ("", path),
        };
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() && !meta.is_dir() => (s, e),
            _ => (name, ""),
        };
        let target = self.available_path(folder, stem, ext);
        let to_abs = self.write_path(&target)?;
        let copied = if meta.is_dir() {
            copy_indexed_tree(&from_abs, &to_abs)
        } else {
            std::fs::copy(&from_abs, &to_abs)
                .map(|_| ())
                .map_err(io("copy", &from_abs))
        };
        // Whatever landed before a failure is on disk, and the list has to
        // say so either way.
        let changes = self.refresh_path(&to_abs);
        copied?;
        Ok((target, changes))
    }

    /// Writes `bytes` as a new attachment `stem.ext` in `folder`, creating
    /// the folder chain, under the first free name [`Vault::available_path`]
    /// gives (`pic one.png`, then `pic one 1.png`): what Obsidian 1.15.0's
    /// Insert attachment and Download attachments both did with a name
    /// already taken. Returns the path written and the index's changes.
    pub fn add_attachment(
        &mut self,
        folder: &str,
        stem: &str,
        ext: &str,
        bytes: &[u8],
    ) -> Result<(String, Vec<FileChange>), OpError> {
        let folder = folder.trim_matches('/');
        let target = self.available_path(folder, stem, ext);
        validate_path(&target)?;
        let abs = self.write_path(&target)?;
        if let Some(parent) = abs.parent() {
            std::fs::create_dir_all(parent).map_err(io("create folder", parent))?;
        }
        crate::write_no_follow(&abs, bytes).map_err(io("write", &abs))?;
        let mut changes = Vec::new();
        // A folder the attachment brought with it is indexed too.
        let mut at = String::new();
        for part in folder.split('/').filter(|p| !p.is_empty()) {
            at = join(&at, part);
            if !self.folders().iter().any(|f| f == &at) {
                let folder_abs = self.absolute(&at);
                changes.extend(self.refresh_path(&folder_abs));
            }
        }
        changes.extend(self.refresh_path(&abs));
        Ok((target, changes))
    }

    /// Files under `path` when it is a folder, or `path` itself when it is a
    /// file in the list.
    fn files_under(&self, path: &str) -> Vec<String> {
        let prefix = format!("{path}/");
        self.files()
            .iter()
            .filter(|f| f.path == path || f.path.starts_with(&prefix))
            .map(|f| f.path.clone())
            .collect()
    }

    /// The files `rename(from, to)` would move, as `(old path, new path)`,
    /// without touching anything.
    pub fn rename_plan(&self, from: &str, to: &str) -> Vec<(String, String)> {
        tft_formats::files::rename_plan(&self.files, from, to)
    }

    /// Whether [`Vault::rename`] would go through, without doing it: the
    /// name, the source, a destination already taken, and a folder moved
    /// into itself. A caller that has somewhere better to put the refusal
    /// than a failed rename asks this first -- the file tree's inline
    /// rename, which keeps its field open on a name the filesystem would
    /// not take.
    pub fn check_rename(&self, from: &str, to: &str) -> Result<(), OpError> {
        let from = from.trim_matches('/');
        let to = to.trim_matches('/');
        validate_path(to)?;
        let from_abs = self.entry_path(from)?;
        if !from_abs.exists() {
            return Err(OpError::NotFound(from.to_string()));
        }
        if from == to {
            return Ok(());
        }
        let to_abs = self.entry_path(to)?;
        // A case-only rename of the same entry is allowed; anything else
        // already at the destination is not.
        let same_entry = from.eq_ignore_ascii_case(to)
            && std::fs::canonicalize(&from_abs).ok() == std::fs::canonicalize(&to_abs).ok();
        if to_abs.exists() && !same_entry {
            return Err(OpError::DestinationExists);
        }
        if to.starts_with(&format!("{from}/")) {
            return Err(OpError::InvalidName("Cannot move a folder into itself."));
        }
        Ok(())
    }

    /// Renames or moves a file or folder. Returns every file that moved as
    /// `(old path, new path)`, so links can be rewritten.
    pub fn rename(&mut self, from: &str, to: &str) -> Result<Vec<(String, String)>, OpError> {
        let from = from.trim_matches('/');
        let to = to.trim_matches('/');
        self.check_rename(from, to)?;
        if from == to {
            return Ok(Vec::new());
        }
        let from_abs = self.entry_path(from)?;
        let to_abs = self.entry_path(to)?;
        if let Some(parent) = to_abs.parent() {
            std::fs::create_dir_all(parent).map_err(io("create folder", parent))?;
        }
        let moved = self.rename_plan(from, to);
        std::fs::rename(&from_abs, &to_abs).map_err(io("rename", &from_abs))?;
        self.refresh_path(&from_abs);
        self.refresh_path(&to_abs);
        Ok(moved)
    }

    /// Deletes a file or folder per `option`. Returns the vault paths of the
    /// files that are gone.
    pub fn trash(&mut self, path: &str, option: TrashOption) -> Result<Vec<String>, OpError> {
        let path = path.trim_matches('/');
        let abs = self.entry_path(path)?;
        if !abs.exists() {
            return Err(OpError::NotFound(path.to_string()));
        }
        let removed = self.files_under(path);
        match option {
            TrashOption::None => {
                if abs.is_dir() {
                    std::fs::remove_dir_all(&abs).map_err(io("delete", &abs))?;
                } else {
                    std::fs::remove_file(&abs).map_err(io("delete", &abs))?;
                }
            }
            TrashOption::Local => self.trash_locally(path)?,
            TrashOption::System => {
                if let Err(err) = system_trash(&abs) {
                    // No system trash here (another platform, or a read-only
                    // data directory): the vault's own trash is the fallback.
                    if err.kind() == std::io::ErrorKind::Unsupported {
                        self.trash_locally(path)?;
                    } else {
                        return Err(OpError::Io {
                            action: "move to the system trash",
                            path: abs,
                            source: err,
                        });
                    }
                }
            }
        }
        self.refresh_path(&abs);
        Ok(removed)
    }

    fn trash_locally(&self, path: &str) -> Result<(), OpError> {
        let abs = self.entry_path(path)?;
        let trash = self.write_path(".trash")?;
        std::fs::create_dir_all(&trash).map_err(io("create folder", &trash))?;
        let name = path.rsplit('/').next().unwrap_or(path);
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() && abs.is_file() => (s, e),
            _ => (name, ""),
        };
        let target = self.numbered_path(".trash", stem, ext, 2);
        let target_abs = self.entry_path(&target)?;
        std::fs::rename(&abs, &target_abs).map_err(io("move to .trash", &abs))
    }
}

/// Copies the folder `from` to `to`, which does not exist yet, as the
/// scanner sees it: regular files and folders, with dot-prefixed entries
/// skipped and links neither followed nor copied. Every file's bytes are
/// copied as they are. (The trash's `copy_tree` below is the other thing:
/// it takes everything, because a trashed folder must come back whole.)
fn copy_indexed_tree(from: &Path, to: &Path) -> Result<(), OpError> {
    let walker = walkdir::WalkDir::new(from)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| !(e.depth() > 0 && e.file_name().to_string_lossy().starts_with('.')));
    for entry in walker {
        let entry = entry.map_err(|e| OpError::Io {
            action: "read",
            path: e
                .path()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| from.to_path_buf()),
            source: e
                .into_io_error()
                .unwrap_or_else(|| std::io::Error::other("walk")),
        })?;
        let rel = entry
            .path()
            .strip_prefix(from)
            .expect("entry under the folder");
        let dest = to.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&dest).map_err(io("create folder", &dest))?;
        } else if entry.file_type().is_file() {
            std::fs::copy(entry.path(), &dest).map_err(io("copy", entry.path()))?;
        }
    }
    Ok(())
}

/// Moves `abs` to the freedesktop.org trash (`$XDG_DATA_HOME/Trash`), writing
/// the `.trashinfo` record the file manager needs to restore it.
#[cfg(all(unix, not(target_os = "macos")))]
fn system_trash(abs: &Path) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .ok_or_else(|| Error::new(ErrorKind::Unsupported, "no home directory"))?;
    let trash = data_home.join("Trash");
    let files = trash.join("files");
    let info = trash.join("info");
    std::fs::create_dir_all(&files)?;
    std::fs::create_dir_all(&info)?;
    let name = abs
        .file_name()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no file name"))?
        .to_string_lossy()
        .into_owned();
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && abs.is_file() => (s.to_string(), format!(".{e}")),
        _ => (name.clone(), String::new()),
    };
    let mut candidate = name.clone();
    let mut n = 2;
    while files.join(&candidate).exists() || info.join(format!("{candidate}.trashinfo")).exists() {
        candidate = format!("{stem} {n}{ext}");
        n += 1;
    }
    let record = format!(
        "[Trash Info]\nPath={}\nDeletionDate={}\n",
        url_encode_path(&abs.to_string_lossy()),
        utc_timestamp()
    );
    std::fs::write(info.join(format!("{candidate}.trashinfo")), record)?;
    let dest = files.join(&candidate);
    match std::fs::rename(abs, &dest) {
        Ok(()) => Ok(()),
        Err(err) if err.raw_os_error() == Some(libc_exdev()) => {
            copy_tree(abs, &dest)?;
            if abs.is_dir() {
                std::fs::remove_dir_all(abs)
            } else {
                std::fs::remove_file(abs)
            }
        }
        Err(err) => {
            let _ = std::fs::remove_file(info.join(format!("{candidate}.trashinfo")));
            Err(err)
        }
    }
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn system_trash(_abs: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "system trash is not implemented on this platform",
    ))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn libc_exdev() -> i32 {
    18
}

#[cfg(all(unix, not(target_os = "macos")))]
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Percent-encodes a path for a `.trashinfo` record, keeping `/` and the
/// unreserved characters.
#[cfg(all(unix, not(target_os = "macos")))]
fn url_encode_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// `YYYY-MM-DDThh:mm:ss` for now, in UTC.
#[cfg(all(unix, not(target_os = "macos")))]
fn utc_timestamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn vault() -> (tempfile::TempDir, Vault) {
        let dir = tempfile::tempdir().unwrap();
        let r = dir.path();
        fs::create_dir_all(r.join("Sub/Deep")).unwrap();
        fs::create_dir_all(r.join("Inbox")).unwrap();
        fs::write(r.join("A.md"), "a").unwrap();
        fs::write(r.join("Untitled.md"), "").unwrap();
        fs::write(r.join("Sub/S.md"), "s").unwrap();
        fs::write(r.join("Sub/Deep/D.md"), "d").unwrap();
        fs::write(r.join("Sub/pic.png"), "p").unwrap();
        let vault = Vault::open(r).unwrap();
        (dir, vault)
    }

    fn paths(v: &Vault) -> Vec<&str> {
        v.files().iter().map(|f| f.path.as_str()).collect()
    }

    /// Insert attachment and Download attachments in Obsidian 1.15.0: the
    /// folder chain is made, a taken name gets the next number before the
    /// extension, and the bytes are the source's.
    #[test]
    fn an_attachment_takes_the_next_free_name_in_a_new_folder() {
        let (dir, mut v) = vault();
        let (first, _) = v
            .add_attachment("Sub/att", "pic one", "png", b"one")
            .unwrap();
        assert_eq!(first, "Sub/att/pic one.png");
        let (second, changes) = v
            .add_attachment("Sub/att", "pic one", "png", b"two")
            .unwrap();
        assert_eq!(second, "Sub/att/pic one 1.png");
        assert_eq!(
            changes,
            vec![FileChange::Added("Sub/att/pic one 1.png".into())]
        );
        assert_eq!(
            fs::read(dir.path().join("Sub/att/pic one 1.png")).unwrap(),
            b"two"
        );
        assert!(v.folders().iter().any(|f| f == "Sub/att"));
        assert!(paths(&v).contains(&"Sub/att/pic one.png"));
        let (root, _) = v.add_attachment("", "pic", "png", b"p").unwrap();
        assert_eq!(root, "pic.png");
        assert!(v.add_attachment("", "bad:name", "png", b"p").is_err());
    }

    #[test]
    fn new_notes_and_folders_take_the_next_free_number() {
        let (_dir, mut v) = vault();
        assert_eq!(v.available_path("", "Untitled", "md"), "Untitled 1.md");
        v.create_note("Untitled 1.md", "").unwrap();
        assert_eq!(v.available_path("", "Untitled", "md"), "Untitled 2.md");
        assert_eq!(v.available_path("", "Untitled", ""), "Untitled");
        v.create_folder("Untitled").unwrap();
        assert_eq!(v.available_path("", "Untitled", ""), "Untitled 1");
        assert_eq!(v.available_path("Sub", "S", "md"), "Sub/S 1.md");
        assert!(matches!(v.create_note("A.md", "x"), Err(OpError::Exists)));
        assert!(matches!(v.create_folder("Sub"), Err(OpError::Exists)));
        assert!(matches!(
            v.create_note("bad:name.md", ""),
            Err(OpError::InvalidName(_))
        ));
        let changes = v.create_note("New/Nested/N.md", "n").unwrap();
        assert_eq!(changes, vec![FileChange::Added("New/Nested/N.md".into())]);
        assert!(paths(&v).contains(&"New/Nested/N.md"));
        assert!(v.folders().contains(&"Untitled".to_string()));
        assert!(v.folders().contains(&"New/Nested".to_string()));
    }

    /// A folder's copy is a sibling named by the same rule as a file's --
    /// `Sub 1`, then `Sub 2` -- holding every file and folder the scanner
    /// would index, byte for byte, with the dot-files the scanner skips left
    /// behind and nothing of the original touched.
    #[test]
    fn a_folder_copy_is_a_byte_identical_sibling() {
        let (dir, mut v) = vault();
        let r = dir.path();
        fs::create_dir_all(r.join("Sub/Empty")).unwrap();
        fs::write(r.join("Sub/Deep/D.md"), "# D\r\n[[S]] and ![[pic.png]]\r\n").unwrap();
        fs::write(
            r.join("Sub/pic.png"),
            [0x89u8, b'P', b'N', b'G', 0, 1, 2, 0xff],
        )
        .unwrap();
        fs::write(r.join("Sub/board.canvas"), "{\"nodes\":[]}").unwrap();
        fs::write(r.join("Sub/.hidden"), "h").unwrap();
        v.rescan().unwrap();
        let originals = [
            "Sub/S.md",
            "Sub/Deep/D.md",
            "Sub/pic.png",
            "Sub/board.canvas",
        ];
        let before: Vec<Vec<u8>> = originals
            .iter()
            .map(|p| fs::read(r.join(p)).unwrap())
            .collect();

        let (copy, changes) = v.duplicate("Sub").unwrap();
        assert_eq!(copy, "Sub 1");
        assert!(
            !copy.starts_with("Sub/"),
            "the copy is a sibling, never inside the original"
        );
        for (p, bytes) in originals.iter().zip(&before) {
            let copied = format!("Sub 1{}", &p["Sub".len()..]);
            assert_eq!(&fs::read(r.join(&copied)).unwrap(), bytes, "{copied}");
            assert_eq!(&fs::read(r.join(p)).unwrap(), bytes, "{p} was touched");
        }
        assert!(r.join("Sub 1/Empty").is_dir(), "empty folders come along");
        assert!(
            !r.join("Sub 1/.hidden").exists(),
            "what the scanner skips is not copied"
        );
        let mut added: Vec<&str> = changes
            .iter()
            .map(|c| {
                assert!(matches!(c, FileChange::Added(_)), "{c:?}");
                c.path()
            })
            .collect();
        added.sort();
        assert_eq!(
            added,
            [
                "Sub 1/Deep/D.md",
                "Sub 1/S.md",
                "Sub 1/board.canvas",
                "Sub 1/pic.png"
            ]
        );
        assert!(v.folders().contains(&"Sub 1/Deep".to_string()));
        assert!(v.folders().contains(&"Sub 1/Empty".to_string()));
        assert_eq!(
            fs::read_dir(r.join("Sub")).unwrap().count(),
            6,
            "the original holds what it held"
        );

        let (second, _) = v.duplicate("Sub").unwrap();
        assert_eq!(second, "Sub 2", "the next free number");
        assert!(r.join("Sub 2/Deep/D.md").is_file());
        let (nested, _) = v.duplicate("Sub/Deep").unwrap();
        assert_eq!(nested, "Sub/Deep 1", "a nested folder copies beside itself");
        assert!(r.join("Sub/Deep 1/D.md").is_file());
    }

    /// A file's copy puts the number before the extension; a folder's name
    /// is never split on a dot.
    #[test]
    fn a_file_copy_numbers_the_stem() {
        let (dir, mut v) = vault();
        let r = dir.path();
        let (copy, changes) = v.duplicate("Sub/S.md").unwrap();
        assert_eq!(copy, "Sub/S 1.md");
        assert_eq!(changes, vec![FileChange::Added("Sub/S 1.md".into())]);
        assert_eq!(fs::read(r.join("Sub/S 1.md")).unwrap(), b"s");
        assert_eq!(v.duplicate("Sub/S.md").unwrap().0, "Sub/S 2.md");
        assert_eq!(v.duplicate("Sub/pic.png").unwrap().0, "Sub/pic 1.png");
        fs::create_dir(r.join("v1.0")).unwrap();
        assert_eq!(v.duplicate("v1.0").unwrap().0, "v1.0 1");
        assert!(matches!(
            v.duplicate("Missing.md"),
            Err(OpError::NotFound(p)) if p == "Missing.md"
        ));
    }

    #[test]
    fn new_note_location_follows_the_setting() {
        let (_dir, v) = vault();
        assert_eq!(v.new_note_folder(None, None, Some("Sub/S.md")), "");
        assert_eq!(
            v.new_note_folder(Some("current"), None, Some("Sub/S.md")),
            "Sub"
        );
        assert_eq!(v.new_note_folder(Some("current"), None, None), "");
        assert_eq!(
            v.new_note_folder(Some("folder"), Some("Inbox"), None),
            "Inbox"
        );
        assert_eq!(v.new_note_folder(Some("folder"), Some("/"), None), "");
        assert_eq!(
            v.new_note_folder(Some("folder"), Some("Missing/Folder"), None),
            "",
            "a missing folder falls back to the root"
        );
        assert_eq!(v.new_note_path("Missing", "Inbox"), "Inbox/Missing.md");
        assert_eq!(
            v.new_note_path("Deep/Also Missing", "Inbox"),
            "Deep/Also Missing.md"
        );
        assert_eq!(v.new_note_path("Note.md", ""), "Note.md");
    }

    /// Every row of the spec's resolution table, asked of a note at the
    /// root and of `Sub/Nested.md`, which is how the probe asked Obsidian.
    #[test]
    fn an_attachment_lands_where_the_setting_says() {
        let table = [
            ("", "photo.png", "photo.png"),
            ("/", "photo.png", "photo.png"),
            ("./", "photo.png", "Sub/photo.png"),
            (
                "./Attachments",
                "Attachments/photo.png",
                "Sub/Attachments/photo.png",
            ),
            (
                "Attachments",
                "Attachments/photo.png",
                "Attachments/photo.png",
            ),
            (
                "att/deep/new",
                "att/deep/new/photo.png",
                "att/deep/new/photo.png",
            ),
        ];
        for (setting, from_root, from_subfolder) in table {
            assert_eq!(
                attachment_path(Some(setting), "Target.md", "photo.png"),
                from_root,
                "{setting:?} from a root note"
            );
            assert_eq!(
                attachment_path(Some(setting), "Sub/Nested.md", "photo.png"),
                from_subfolder,
                "{setting:?} from Sub/Nested.md"
            );
        }
    }

    /// The key is absent until something sets it, and then it is the root,
    /// as `""` is. The folder on its own is what a caller creating the
    /// chain needs.
    #[test]
    fn an_absent_attachment_folder_is_the_vault_root() {
        assert_eq!(
            attachment_path(None, "Sub/Nested.md", "photo.png"),
            "photo.png"
        );
        assert_eq!(attachment_folder(None, "Sub/Nested.md"), "");
        assert_eq!(
            attachment_folder(Some("./"), "Sub/Deep/Note.md"),
            "Sub/Deep"
        );
        assert_eq!(
            attachment_folder(Some("./Attachments/"), "Sub/Nested.md"),
            "Sub/Attachments"
        );
        assert_eq!(
            attachment_folder(Some("/Attachments/"), "Note.md"),
            "Attachments"
        );
    }

    #[test]
    fn rename_moves_files_and_folders_and_reports_every_file() {
        let (dir, mut v) = vault();
        assert_eq!(
            v.rename("A.md", "Renamed A.md").unwrap(),
            vec![("A.md".to_string(), "Renamed A.md".to_string())]
        );
        assert!(dir.path().join("Renamed A.md").is_file());
        assert!(!paths(&v).contains(&"A.md"));
        assert!(paths(&v).contains(&"Renamed A.md"));
        let moved = v.rename("Sub", "Other/Sub2").unwrap();
        assert_eq!(
            moved,
            vec![
                (
                    "Sub/Deep/D.md".to_string(),
                    "Other/Sub2/Deep/D.md".to_string()
                ),
                ("Sub/S.md".to_string(), "Other/Sub2/S.md".to_string()),
                ("Sub/pic.png".to_string(), "Other/Sub2/pic.png".to_string()),
            ]
        );
        assert_eq!(
            paths(&v),
            vec![
                "Other/Sub2/Deep/D.md",
                "Other/Sub2/S.md",
                "Other/Sub2/pic.png",
                "Renamed A.md",
                "Untitled.md"
            ]
        );
        assert_eq!(
            v.folders(),
            ["Inbox", "Other", "Other/Sub2", "Other/Sub2/Deep"]
        );
        assert!(matches!(
            v.rename("Untitled.md", "Renamed A.md"),
            Err(OpError::DestinationExists)
        ));
        assert!(matches!(
            v.rename("Other", "Other/Sub2/X"),
            Err(OpError::InvalidName(_))
        ));
        assert!(matches!(
            v.rename("Nope.md", "X.md"),
            Err(OpError::NotFound(_))
        ));
        // A case-only rename keeps the same file.
        v.rename("Renamed A.md", "renamed a.md").unwrap();
        assert!(paths(&v).contains(&"renamed a.md"));
        assert!(v.rename("renamed a.md", "renamed a.md").unwrap().is_empty());
    }

    /// The file tree's inline rename asks before it closes its field, so
    /// `check_rename` has to give the same answer `rename` would and
    /// change nothing while it does: a refused name leaves the field open
    /// over a vault that still looks exactly as it did.
    #[test]
    fn check_rename_answers_for_rename_without_touching_anything() {
        let (_dir, mut v) = vault();
        let before = paths(&v).join("|");
        assert!(matches!(
            v.check_rename("A.md", "Untitled.md"),
            Err(OpError::DestinationExists)
        ));
        assert!(matches!(
            v.check_rename("A.md", ""),
            Err(OpError::InvalidName(_))
        ));
        assert!(matches!(
            v.check_rename("Nope.md", "X.md"),
            Err(OpError::NotFound(_))
        ));
        assert!(matches!(
            v.check_rename("Inbox", "Inbox/Deeper"),
            Err(OpError::InvalidName(_))
        ));
        // The allowed ones: a free name, a case-only rename of the same
        // file, and a rename onto itself.
        assert!(v.check_rename("A.md", "Free.md").is_ok());
        assert!(v.check_rename("A.md", "a.md").is_ok());
        assert!(v.check_rename("A.md", "A.md").is_ok());
        assert_eq!(paths(&v).join("|"), before, "asking must not move a byte");
        // And what it allows, `rename` does.
        v.rename("A.md", "Free.md").unwrap();
        assert!(paths(&v).contains(&"Free.md"));
    }

    #[test]
    fn trash_goes_to_the_local_folder_with_numbered_collisions() {
        let (dir, mut v) = vault();
        fs::create_dir_all(dir.path().join(".trash")).unwrap();
        fs::write(dir.path().join(".trash/A.md"), "old").unwrap();
        assert_eq!(v.trash("A.md", TrashOption::Local).unwrap(), vec!["A.md"]);
        assert!(dir.path().join(".trash/A 2.md").is_file());
        v.create_note("A.md", "again").unwrap();
        v.trash("A.md", TrashOption::Local).unwrap();
        assert!(dir.path().join(".trash/A 3.md").is_file());
        let removed = v.trash("Sub", TrashOption::Local).unwrap();
        assert_eq!(removed, vec!["Sub/Deep/D.md", "Sub/S.md", "Sub/pic.png"]);
        assert!(dir.path().join(".trash/Sub/Deep/D.md").is_file());
        assert_eq!(paths(&v), vec!["Untitled.md"]);
        assert_eq!(v.folders(), ["Inbox"]);
        v.trash("Untitled.md", TrashOption::None).unwrap();
        assert!(paths(&v).is_empty());
        assert!(matches!(
            v.trash("Untitled.md", TrashOption::None),
            Err(OpError::NotFound(_))
        ));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn freedesktop_records_are_well_formed() {
        assert_eq!(
            url_encode_path("/home/me/My Vault/Ä.md"),
            "/home/me/My%20Vault/%C3%84.md"
        );
        let stamp = utc_timestamp();
        assert_eq!(stamp.len(), 19);
        assert_eq!(&stamp[10..11], "T");
        assert!(stamp.starts_with("20"));
    }
}
