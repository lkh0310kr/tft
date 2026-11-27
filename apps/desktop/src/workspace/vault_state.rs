use crate::reading;
use crate::settings::SettingChange;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock, RwLockReadGuard};
use std::time::Instant;
use tft_cache::{CachedMetadata, LinkFormat, MetadataCache, index_markdown};
use tft_formats::FileKind;
use tft_formats::config::{
    AppConfig, AppearanceConfig, PagePreviewConfig, PropertyTypes, SwitcherConfig,
};
use tft_search::ExcludeFilters;
use tft_vault::{FileChange, TrashOption, Vault, VaultError};

/// Everything derived from the vault folder, built off the UI thread and kept current by `apply`.
pub struct VaultState {
    pub root: PathBuf,
    pub(super) vault: RwLock<Vault>,
    /// The same vault as a node graph; the desktop still reads `vault`, cache
    /// and texts by path while it moves over.
    pub(super) data: RwLock<tft_app::VaultData>,
    pub(super) cache: RwLock<MetadataCache>,
    /// Note text as indexed, for search and mentions.
    pub(super) texts: RwLock<HashMap<String, Arc<String>>>,
    /// Config files Settings writes, locked because a control changes one while the vault is open.
    pub(super) app: RwLock<AppConfig>,
    /// `app.json`'s Excluded files, compiled whenever `app` is set.
    pub(super) excluded: RwLock<ExcludeFilters>,
    pub(super) appearance: RwLock<AppearanceConfig>,
    pub(super) switcher: RwLock<SwitcherConfig>,
    /// `page-preview.json`, read by the hover preview's sources.
    pub(super) page_preview: RwLock<PagePreviewConfig>,
    /// `types.json`: the type assigned to each property name.
    pub(super) property_types: RwLock<PropertyTypes>,
    /// Config files that would not parse; the app runs on defaults for them
    /// and refuses to write over what the user had.
    pub unreadable_configs: Vec<String>,
    pub index_ms: f64,
}

/// One re-read note: its path and, for markdown, the new record and text.
type Reindexed = (String, Option<(CachedMetadata, Arc<String>)>);

/// What a batch of filesystem events changed.
#[derive(Debug, Default)]
pub struct Changes {
    pub files: Vec<FileChange>,
    pub notes: Vec<String>,
    pub removed_notes: Vec<String>,
}

impl Changes {
    pub(super) fn structural(&self) -> bool {
        self.files
            .iter()
            .any(|c| !matches!(c, FileChange::Modified(_)))
    }
}

/// What a rename did: the files that moved, the links rewritten, and the index changes.
#[derive(Debug, Default)]
pub struct RenameOutcome {
    pub changes: Changes,
    pub renamed: Vec<(String, String)>,
    pub rewritten_files: usize,
    pub rewritten_links: usize,
    /// Notes whose links were left alone because the file changed underneath the rewrite.
    pub stale: Vec<String>,
}

impl VaultState {
    pub fn load(root: &PathBuf) -> Result<VaultState, String> {
        let started = Instant::now();
        let vault = Vault::open(root).map_err(|e| e.to_string())?;
        // A config that will not parse falls back to defaults so one broken
        // file cannot stop a vault opening; the window reports it and writes are refused.
        let mut unreadable = Vec::new();
        // Only a parse failure counts: an unreadable file has nothing on disk to lose.
        let mut note = |name: &'static str, err: Option<&VaultError>| {
            if matches!(err, Some(VaultError::Json { .. })) {
                unreadable.push(name.to_string());
            }
        };
        let app = vault.app_config();
        note("app.json", app.as_ref().err());
        let app = app.unwrap_or_default();
        let appearance = vault.appearance_config();
        note("appearance.json", appearance.as_ref().err());
        let appearance = appearance.unwrap_or_default();
        let switcher = vault.switcher_config();
        note("switcher.json", switcher.as_ref().err());
        let switcher = switcher.unwrap_or_default();
        let page_preview = vault.page_preview_config();
        note("page-preview.json", page_preview.as_ref().err());
        let page_preview = page_preview.unwrap_or_default();
        let property_types = vault.property_types();
        note("types.json", property_types.as_ref().err());
        let property_types = property_types.unwrap_or_default();
        let notes: Vec<String> = vault.markdown_files().map(|f| f.path.clone()).collect();
        let loaded: Vec<(String, CachedMetadata, Arc<String>)> = notes
            .par_iter()
            .map(|path| {
                let text = std::fs::read_to_string(vault.absolute(path)).unwrap_or_default();
                let meta = index_markdown(&text);
                (path.clone(), meta, Arc::new(text))
            })
            .collect();
        let mut texts = HashMap::with_capacity(loaded.len());
        let metas: Vec<(String, CachedMetadata)> = loaded
            .into_iter()
            .map(|(path, meta, text)| {
                texts.insert(path.clone(), text);
                (path, meta)
            })
            .collect();
        let cache = MetadataCache::new(vault.link_resolver(), metas);
        let mut data = tft_app::VaultData::new(vault.root().to_string_lossy());
        vault.apply_snapshot(&mut data);
        let state = VaultState {
            root: vault.root().to_path_buf(),
            vault: RwLock::new(vault),
            data: RwLock::new(data),
            cache: RwLock::new(cache),
            texts: RwLock::new(texts),
            excluded: RwLock::new(ExcludeFilters::new(&[] as &[String])),
            app: RwLock::new(app),
            appearance: RwLock::new(appearance),
            switcher: RwLock::new(switcher),
            page_preview: RwLock::new(page_preview),
            property_types: RwLock::new(property_types),
            unreadable_configs: unreadable,
            index_ms: started.elapsed().as_secs_f64() * 1000.0,
        };
        state.verify_data()?;
        Ok(state)
    }

    pub fn property_types(&self) -> RwLockReadGuard<'_, PropertyTypes> {
        self.property_types.read().expect("property types lock")
    }

    // Every core plugin is on; `core-plugins.json` is not read.
    pub fn core_plugin_enabled(&self, _id: &str) -> bool {
        true
    }

    pub fn app(&self) -> RwLockReadGuard<'_, AppConfig> {
        self.app.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The Excluded files rules: what search, the graph, unlinked mentions and
    /// the tags pane leave out, and what the switcher puts last.
    pub fn excluded(&self) -> RwLockReadGuard<'_, ExcludeFilters> {
        self.excluded.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_excluded(&self, path: &str) -> bool {
        self.excluded().is_excluded(path)
    }

    pub fn appearance(&self) -> RwLockReadGuard<'_, AppearanceConfig> {
        self.appearance.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Re-reads `appearance.json` from disk into the config in hand and
    /// answers it: an edit made outside the app counts as much as one of
    /// its own.
    pub fn reload_appearance(&self) -> AppearanceConfig {
        let fresh = self
            .vault
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .appearance_config()
            .ok();
        let mut cur = self.appearance.write().unwrap_or_else(|e| e.into_inner());
        if let Some(fresh) = fresh {
            *cur = fresh;
        }
        cur.clone()
    }

    pub fn config_dir(&self) -> String {
        self.vault
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .config_dir()
            .to_string()
    }

    pub fn switcher(&self) -> RwLockReadGuard<'_, SwitcherConfig> {
        self.switcher.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn page_preview(&self) -> RwLockReadGuard<'_, PagePreviewConfig> {
        self.page_preview.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Writes a setting the Settings window changed and keeps the config in step;
    /// the file goes first, since a change that cannot reach disk does not count.
    pub fn apply_setting(&self, change: &SettingChange) -> Result<(), String> {
        match change {
            SettingChange::App(config) => {
                self.write_config("app.json", config)?;
                *self.app.write().unwrap_or_else(|e| e.into_inner()) = config.clone();
            }
            SettingChange::Appearance(config) => {
                self.write_config("appearance.json", config)?;
                *self.appearance.write().unwrap_or_else(|e| e.into_inner()) = config.clone();
            }
        }
        Ok(())
    }

    fn write_config<T: serde::Serialize>(&self, name: &str, value: &T) -> Result<(), String> {
        self.vault()
            .write_config(name, value)
            .map_err(|e| e.to_string())
    }

    /// Records `key`'s type in `types.json`, as Obsidian's type menu does, and returns the table.
    pub fn assign_property_type(
        &self,
        key: &str,
        ty: tft_formats::properties::PropertyType,
    ) -> Result<PropertyTypes, String> {
        let mut types = self.property_types.write().expect("property types lock");
        // Obsidian seeds the built-in keys on every write, before the key being assigned.
        types.seed_defaults();
        types.types.insert(key.to_string(), ty.as_str().to_string());
        self.vault()
            .write_config("types.json", &*types)
            .map_err(|e| e.to_string())?;
        Ok(types.clone())
    }

    pub fn vault(&self) -> RwLockReadGuard<'_, Vault> {
        self.vault.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn data(&self) -> RwLockReadGuard<'_, tft_app::VaultData> {
        self.data.read().unwrap_or_else(|e| e.into_inner())
    }

    /// Rebuilds the node graph from the path vault, reusing backend IDs so
    /// nodes that did not move keep their NodeId.
    fn sync_data(&self) {
        let vault = self.vault();
        let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
        vault.apply_snapshot(&mut data);
    }

    /// The desktop migration's ratchet: every unique path the path vault
    /// reports must have exactly one node, and the projections must agree.
    pub fn verify_data(&self) -> Result<(), String> {
        let vault = self.vault();
        let data = self.data();
        let expected_files: std::collections::BTreeSet<&str> = vault
            .files()
            .iter()
            .map(|f| f.path.as_str())
            .filter(|p| data.lookup(p).is_ok())
            .collect();
        let actual_files: std::collections::BTreeSet<String> =
            data.files().into_iter().map(|f| f.path).collect();
        if expected_files.len() != actual_files.len()
            || !expected_files.iter().all(|p| actual_files.contains(*p))
        {
            return Err(format!(
                "files differ: {} in the vault, {} in the node graph",
                expected_files.len(),
                actual_files.len()
            ));
        }
        let expected_folders: std::collections::BTreeSet<&str> = vault
            .folders()
            .iter()
            .map(String::as_str)
            .filter(|p| data.lookup(p).is_ok())
            .collect();
        let actual_folders: std::collections::BTreeSet<String> =
            data.folders().into_iter().collect();
        if expected_folders.len() != actual_folders.len()
            || !expected_folders.iter().all(|p| actual_folders.contains(*p))
        {
            return Err(format!(
                "folders differ: {} in the vault, {} in the node graph",
                expected_folders.len(),
                actual_folders.len()
            ));
        }
        data.validate()
            .map_err(|e| format!("node graph invalid: {e:?}"))
    }

    pub fn cache(&self) -> RwLockReadGuard<'_, MetadataCache> {
        self.cache.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn texts(&self) -> RwLockReadGuard<'_, HashMap<String, Arc<String>>> {
        self.texts.read().unwrap_or_else(|e| e.into_inner())
    }

    pub fn build_options(&self) -> reading::BuildOptions {
        reading::BuildOptions {
            strict_line_breaks: crate::fixed::STRICT_LINE_BREAKS,
        }
    }

    /// Applies filesystem events for `paths`: refreshes the file list, re-indexes
    /// changed notes and re-resolves links when files came or went.
    pub fn apply(&self, paths: &[PathBuf]) -> Changes {
        let mut files = Vec::new();
        {
            let mut vault = self.vault.write().unwrap_or_else(|e| e.into_inner());
            for path in paths {
                files.extend(vault.refresh_path(path));
            }
        }
        self.absorb(files)
    }

    /// Re-reads and re-indexes the notes among `files` and re-resolves links when files came or went.
    pub(super) fn absorb(&self, files: Vec<FileChange>) -> Changes {
        if files.iter().any(|c| !matches!(c, FileChange::Modified(_))) {
            self.sync_data();
        }
        let mut changes = Changes::default();
        // Keep one change per path, the last one seen.
        let mut last: HashMap<String, FileChange> = HashMap::new();
        for c in files {
            last.insert(c.path().to_string(), c);
        }
        let mut files: Vec<FileChange> = last.into_values().collect();
        files.sort_by(|a, b| a.path().cmp(b.path()));
        if files.is_empty() {
            return changes;
        }
        let reread: Vec<Reindexed> = files
            .par_iter()
            .map(|c| match c {
                FileChange::Added(p) | FileChange::Modified(p)
                    if FileKind::from_path(p) == FileKind::Markdown =>
                {
                    let text = std::fs::read_to_string(self.root.join(p)).unwrap_or_default();
                    let meta = index_markdown(&text);
                    (p.clone(), Some((meta, Arc::new(text))))
                }
                other => (other.path().to_string(), None),
            })
            .collect();
        {
            let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
            let mut texts = self.texts.write().unwrap_or_else(|e| e.into_inner());
            for (c, (path, indexed)) in files.iter().zip(reread) {
                match (c, indexed) {
                    (FileChange::Removed(_), _) => {
                        if texts.remove(&path).is_some() {
                            changes.removed_notes.push(path.clone());
                        }
                        cache.forget_path(&path);
                    }
                    (_, Some((meta, text))) => {
                        cache.insert_file(path.clone(), meta);
                        texts.insert(path.clone(), text);
                        changes.notes.push(path);
                    }
                    (_, None) => cache.add_path(&path),
                }
            }
            if files.iter().any(|c| !matches!(c, FileChange::Modified(_))) {
                cache.reresolve();
            }
        }
        changes.files = files;
        changes
    }

    pub fn link_format(&self) -> LinkFormat {
        LinkFormat::Shortest
    }

    pub fn trash_option(&self) -> TrashOption {
        TrashOption::System
    }

    /// `alwaysUpdateLinks`, **off** by default: a rename asks before rewriting.
    /// Obsidian's row says "Turn off to be prompted to update links after renaming
    /// a file", and the toggle rendered off on a vault whose `app.json` was `{}`.
    pub fn update_links_automatically(&self) -> bool {
        false
    }

    /// "Confirm before deleting files" left the Settings window, fixed on.
    pub fn confirm_delete(&self) -> bool {
        crate::fixed::PROMPT_DELETE
    }

    /// Where a note created for the link text `name` goes (`newFileLocation`, `newFileFolderPath`).
    pub fn new_note_path(&self, name: &str, _current: Option<&str>) -> String {
        self.vault().new_note_path(name, "")
    }

    /// Creates a note (and its folders) and indexes it.
    pub fn create_note(&self, path: &str, text: &str) -> Result<Changes, String> {
        let files = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .create_note(path, text)
            .map_err(|e| e.to_string())?;
        Ok(self.absorb(files))
    }

    pub fn create_folder(&self, path: &str) -> Result<(), String> {
        self.vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .create_folder(path)
            .map_err(|e| e.user_message())?;
        self.sync_data();
        Ok(())
    }

    /// Copies a file or folder next to itself (`Name 1.ext`, `Name 1`) and indexes the copy.
    pub fn duplicate(&self, path: &str) -> Result<(String, Changes), String> {
        let (target, files) = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .duplicate(path)
            .map_err(|e| e.user_message())?;
        Ok((target, self.absorb(files)))
    }

    /// Whether the rename would go through, without doing it.
    pub fn check_rename(&self, from: &str, to: &str) -> Result<(), String> {
        self.vault()
            .check_rename(from, to)
            .map_err(|e| e.user_message())
    }

    /// The rewrites a rename would need, without doing anything: `(files, links)`.
    pub fn rename_preview(&self, from: &str, to: &str) -> (usize, usize) {
        let plan = self.vault().rename_plan(from, to);
        let texts = self.texts();
        let rewrites = self
            .cache()
            .plan_rename_rewrites(&plan, self.link_format(), &|p| {
                texts.get(p).map(|t| t.to_string())
            });
        (
            rewrites.len(),
            rewrites.iter().map(|r| r.links).sum::<usize>(),
        )
    }

    /// Writes a block id into `path`, a note the user is not editing. The bytes
    /// on disk must be the bytes the insertion was planned against; the watcher re-indexes.
    pub fn write_block_id(
        &self,
        path: &str,
        before: &str,
        insertion: &tft_suggest::blocks::IdInsertion,
    ) -> Result<(), String> {
        let absolute = self.vault().write_path(path).map_err(|e| e.to_string())?;
        let on_disk = std::fs::read_to_string(&absolute)
            .map_err(|e| format!("Could not read {path}: {e}"))?;
        if on_disk != before {
            return Err(format!("{path} changed on disk; no id was written"));
        }
        tft_vault::write_no_follow(&absolute, insertion.applied(&on_disk).as_bytes())
            .map_err(|e| format!("Could not write {path}: {e}"))
    }

    /// Renames or moves a file or folder, rewrites links to it when `update_links`,
    /// and brings the index up to date.
    pub fn rename(
        &self,
        from: &str,
        to: &str,
        update_links: bool,
    ) -> Result<RenameOutcome, String> {
        let moved = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .rename(from, to)
            // Both of these end up in a modal the user typed into, so they
            // get the sentence rather than the path and the errno.
            .map_err(|e| e.user_message())?;
        {
            let mut data = self.data.write().unwrap_or_else(|e| e.into_inner());
            data.rebase_local_prefix(from, to);
            for (old, new) in &moved {
                data.rebase_local_prefix(old, new);
            }
        }
        self.sync_data();
        let plan = if update_links {
            let texts = self.texts();
            self.cache()
                .plan_rename_rewrites(&moved, self.link_format(), &|p| {
                    texts.get(p).map(|t| t.to_string())
                })
        } else {
            Vec::new()
        };
        // Each rewrite was planned from the indexed text, so the file must still
        // be that text: a note written underneath keeps its own bytes and its
        // links are left alone rather than overwritten.
        let mut stale: Vec<String> = Vec::new();
        let texts = self.texts();
        for rw in &plan {
            let absolute = self
                .vault()
                .write_path(&rw.new_path)
                .map_err(|e| e.to_string())?;
            let planned_from = texts.get(&rw.old_path).map(|t| t.to_string());
            match (std::fs::read_to_string(&absolute), planned_from) {
                (Ok(on_disk), Some(was)) if on_disk != was => {
                    stale.push(rw.new_path.clone());
                    continue;
                }
                _ => {}
            }
            tft_vault::write_no_follow(&absolute, rw.text.as_bytes())
                .map_err(|e| format!("Could not update links in {}: {e}", rw.new_path))?;
        }
        drop(texts);
        let moved_from: HashMap<&str, &str> = moved
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let mut changes = Changes::default();
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let mut texts = self.texts.write().unwrap_or_else(|e| e.into_inner());
        for (old, new) in &moved {
            let old_text = texts.remove(old);
            cache.forget_path(old);
            changes.files.push(FileChange::Removed(old.clone()));
            if old_text.is_some() {
                changes.removed_notes.push(old.clone());
            }
            if FileKind::from_path(new) == FileKind::Markdown {
                let text = plan
                    .iter()
                    .find(|r| r.old_path == *old)
                    .map(|r| Arc::new(r.text.clone()))
                    .or(old_text)
                    .unwrap_or_else(|| {
                        Arc::new(std::fs::read_to_string(self.root.join(new)).unwrap_or_default())
                    });
                cache.insert_file(new.clone(), index_markdown(&text));
                texts.insert(new.clone(), text);
                changes.notes.push(new.clone());
            } else {
                cache.add_path(new);
            }
            changes.files.push(FileChange::Added(new.clone()));
        }
        for rw in plan
            .iter()
            .filter(|r| !moved_from.contains_key(r.old_path.as_str()))
        {
            let text = Arc::new(rw.text.clone());
            cache.insert_file(rw.new_path.clone(), index_markdown(&text));
            texts.insert(rw.new_path.clone(), text);
            changes
                .files
                .push(FileChange::Modified(rw.new_path.clone()));
            changes.notes.push(rw.new_path.clone());
        }
        cache.reresolve();
        Ok(RenameOutcome {
            changes,
            renamed: moved,
            rewritten_files: plan.len() - stale.len(),
            rewritten_links: plan
                .iter()
                .filter(|r| !stale.contains(&r.new_path))
                .map(|r| r.links)
                .sum(),
            stale,
        })
    }

    /// Writes the notes a heading rename rewrites and indexes them, leaving a note
    /// that changed underneath alone; returns the changes and the stale paths.
    pub fn write_link_rewrites(
        &self,
        plan: &[tft_cache::Rewrite],
    ) -> Result<(Changes, Vec<String>), String> {
        let mut stale: Vec<String> = Vec::new();
        let mut written: Vec<&tft_cache::Rewrite> = Vec::new();
        for rw in plan {
            let absolute = self
                .vault()
                .write_path(&rw.new_path)
                .map_err(|e| e.to_string())?;
            let planned_from = self.texts().get(&rw.old_path).map(|t| t.to_string());
            match (std::fs::read_to_string(&absolute), planned_from) {
                (Ok(on_disk), Some(was)) if on_disk == was => {}
                _ => {
                    stale.push(rw.new_path.clone());
                    continue;
                }
            }
            tft_vault::write_no_follow(&absolute, rw.text.as_bytes())
                .map_err(|e| format!("Could not update links in {}: {e}", rw.new_path))?;
            written.push(rw);
        }
        let mut changes = Changes::default();
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let mut texts = self.texts.write().unwrap_or_else(|e| e.into_inner());
        for rw in written {
            let text = Arc::new(rw.text.clone());
            cache.insert_file(rw.new_path.clone(), index_markdown(&text));
            texts.insert(rw.new_path.clone(), text);
            changes
                .files
                .push(FileChange::Modified(rw.new_path.clone()));
            changes.notes.push(rw.new_path.clone());
        }
        cache.reresolve();
        Ok((changes, stale))
    }

    /// Deletes a file or folder per `trashOption` and drops it from the index.
    pub fn trash(&self, path: &str) -> Result<Changes, String> {
        let removed = self
            .vault
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .trash(path, self.trash_option())
            .map_err(|e| e.to_string())?;
        self.sync_data();
        let mut changes = Changes::default();
        let mut cache = self.cache.write().unwrap_or_else(|e| e.into_inner());
        let mut texts = self.texts.write().unwrap_or_else(|e| e.into_inner());
        for p in removed {
            if texts.remove(&p).is_some() {
                changes.removed_notes.push(p.clone());
            }
            cache.forget_path(&p);
            changes.files.push(FileChange::Removed(p));
        }
        cache.reresolve();
        Ok(changes)
    }
}

impl reading::EmbedSource for VaultState {
    fn resolve(&self, linktext: &str, source_path: &str) -> Option<String> {
        self.cache()
            .resolve(linktext, source_path)
            .map(str::to_string)
    }

    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.vault().absolute(path)).ok()
    }

    fn absolute(&self, path: &str) -> PathBuf {
        self.vault().absolute(path)
    }
}
