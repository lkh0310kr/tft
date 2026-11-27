//! Desktop handling of `tft://` links for startup and the control socket.

use super::Workspace;
use gpui::{Context, Window};

impl Workspace {
    /// An `tft://` URI, from the command line or the control socket:
    /// see [`crate::uri`]. A URI naming another vault is refused.
    pub fn handle_uri(
        &mut self,
        uri: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        use crate::uri::{names_vault, Uri};
        let parsed = crate::uri::parse(uri)?;
        let Some(state) = self.vault_session.state.clone() else {
            return Err("no vault is open".to_string());
        };
        let vault_name = state
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let vault = match &parsed {
            Uri::Open { vault, .. }
            | Uri::New { vault, .. }
            | Uri::Search { vault, .. }
            | Uri::Daily { vault } => vault.clone(),
        };
        if !names_vault(vault.as_deref(), &vault_name) {
            return Err(format!(
                "the URI names the vault {}; this window shows {vault_name}",
                vault.unwrap_or_default()
            ));
        }
        match parsed {
            Uri::Open { file, path, .. } => {
                if let Some(path) = path {
                    let exists = state.vault().absolute(&path).exists();
                    let path = if exists || path.contains('.') {
                        path
                    } else {
                        format!("{path}.md")
                    };
                    self.open_file(path, None, cx);
                } else if let Some(name) = file {
                    if !self.open_by_name(&name, cx) {
                        return Err(format!("no note named {name}"));
                    }
                }
                Ok(())
            }
            Uri::New {
                name,
                path,
                content,
                silent,
                append,
                prepend,
                overwrite,
                ..
            } => {
                let path = match (path, name) {
                    (Some(path), _) => path,
                    (None, Some(name)) => format!("{}.md", name.trim_end_matches(".md")),
                    (None, None) => "Untitled.md".to_string(),
                };
                let content = content.unwrap_or_default();
                let absolute = state.vault().absolute(&path);
                if absolute.exists() {
                    let on_disk = std::fs::read_to_string(&absolute).map_err(|e| e.to_string())?;
                    let text = if overwrite {
                        Some(content)
                    } else if append {
                        Some(format!("{on_disk}{content}"))
                    } else if prepend {
                        Some(format!("{content}{on_disk}"))
                    } else {
                        None
                    };
                    if let Some(text) = text {
                        if self.buffer_differs(&path, &on_disk, cx) {
                            return Err(format!("{path} has unsaved edits"));
                        }
                        tft_vault::write_no_follow(&absolute, text.as_bytes())
                            .map_err(|e| e.to_string())?;
                        self.files_changed(vec![absolute], cx);
                    }
                } else {
                    let changes = state.create_note(&path, &content)?;
                    self.after_changes(changes, cx);
                }
                if !silent {
                    self.open_file(path, None, cx);
                }
                Ok(())
            }
            Uri::Search { query, .. } => {
                self.show_search(&query.unwrap_or_default(), Some(window), cx);
                Ok(())
            }
            Uri::Daily { .. } => Ok(()),
        }
    }
}
