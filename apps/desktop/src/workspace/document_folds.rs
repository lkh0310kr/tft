//! Fold state shared between the Desktop editor/reader views and the local vault store.

use super::{VaultState, Workspace};
use crate::fold;
use gpui::Context;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tft_formats::FileKind;

/// Lazily opened fold records for the currently used local vault.
#[derive(Default)]
pub(super) struct FoldStoreState {
    store: Option<(PathBuf, fold::FoldStore)>,
}

impl FoldStoreState {
    pub(super) fn store(&mut self, root: &Path) -> Option<&mut fold::FoldStore> {
        if self.store.as_ref().is_none_or(|(path, _)| path != root) {
            self.store = Some((root.to_path_buf(), fold::FoldStore::open(root)));
        }
        self.store.as_mut().map(|(_, store)| store)
    }
}

impl Workspace {
    /// Hands the folds across to the editor when `to_editor`, else to the reading view.
    pub(super) fn sync_folds(&mut self, to_editor: bool, cx: &mut Context<Self>) {
        if to_editor {
            let (keys, props) = {
                let r = self.active_reading().read(cx);
                (r.fold_keys(), r.properties_folded())
            };
            self.active_editor().update(cx, |e, cx| {
                e.set_fold_keys(keys, cx);
                e.set_properties_folded(props, cx);
            });
        } else {
            let (keys, props) = {
                let e = self.active_editor().read(cx);
                (e.fold_keys(), e.properties_folded())
            };
            self.active_reading().update(cx, |r, cx| {
                r.set_fold_keys(keys, cx);
                r.set_properties_folded(props, cx);
            });
        }
    }

    /// Writes the open note's folds to the store: on leaving and on quit; the reading
    /// view's go to the editor first, which knows the extents.
    pub(super) fn save_folds(&mut self, path: &str, cx: &mut Context<Self>) {
        if FileKind::from_path(path) != FileKind::Markdown {
            return;
        }
        if self.active_editor().read(cx).name.as_ref() != path {
            return;
        }
        if !self.active_group_state.mode.is_editing() {
            self.sync_folds(true, cx);
        }
        let Some(record) = self.active_editor().read(cx).fold_record() else {
            return;
        };
        let Some(root) = self.vault_session.state.as_ref().map(|s| s.root.clone()) else {
            return;
        };
        if let Some(store) = self.fold_store.store(&root) {
            store.put(path, record);
        }
    }

    /// Restores a note's folds as it opens: the editor takes the `from` lines, reading
    /// the keys they come to.
    pub(super) fn restore_folds(
        &mut self,
        path: &str,
        text: &str,
        state: &Arc<VaultState>,
        cx: &mut Context<Self>,
    ) {
        let Some(record) = self
            .fold_store
            .store(&state.root)
            .and_then(|store| store.get(path).cloned())
        else {
            return;
        };
        let (headings, lists) = (crate::fixed::FOLD_HEADINGS, crate::fixed::FOLD_INDENTS);
        let doc = tft_markdown::parse(text);
        let foldables = fold::foldables(&doc, text, fold::FoldOptions { headings, lists });
        let froms = record
            .folds
            .iter()
            .filter(|r| **r != fold::PROPERTIES_RANGE)
            .map(|r| r.from);
        let keys = fold::Folds::from_record(&foldables, froms).keys();
        let props = record.folds.contains(&fold::PROPERTIES_RANGE);
        self.active_editor()
            .update(cx, |e, cx| e.set_fold_record(&record, cx));
        self.active_reading().update(cx, |r, cx| {
            r.set_fold_keys(keys, cx);
            r.set_properties_folded(props, cx);
        });
    }
}
