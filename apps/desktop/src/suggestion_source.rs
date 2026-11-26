use crate::workspace::VaultState;
use std::sync::Arc;
use tft_app::editor::{SuggestionInsertion, SuggestionSource};
use tft_suggest::blocks;
use tft_suggest::{self as suggest, Accept, BlockId, Item, Suggestion, Triggered, Vault};

/// Adapts the Desktop's live vault/index and safe block-ID writes to the shared editor contract.
pub struct DesktopSuggestionSource {
    state: Arc<VaultState>,
}

impl DesktopSuggestionSource {
    pub fn new(state: Arc<VaultState>) -> Self {
        Self { state }
    }

    fn with_vault<R>(&self, source: &str, f: impl FnOnce(&Vault<'_>) -> R) -> R {
        let vault = self.state.vault();
        let cache = self.state.cache();
        let texts = |path: &str| self.state.texts().get(path).map(|text| text.to_string());
        f(&Vault {
            files: vault.files(),
            cache: &cache,
            source,
            show_unsupported: tft_app::fixed::SHOW_UNSUPPORTED_FILES,
            text: &texts,
            excluded: &self.state.excluded(),
        })
    }
}

impl SuggestionSource for DesktopSuggestionSource {
    fn suggestions(&self, trigger: &Triggered<'_>, source: &str) -> Vec<Suggestion> {
        self.with_vault(source, |vault| suggest::suggestions(trigger, vault))
    }

    fn insert(
        &self,
        trigger: &Triggered<'_>,
        item: &Item,
        accept: Accept,
        source: &str,
        current_text: &str,
        options: suggest::Options,
        crlf: bool,
    ) -> Result<SuggestionInsertion, String> {
        let mut item = item.clone();
        let mut own_note = None;
        if let Item::Block {
            path,
            id: BlockId::New { end, placement },
        } = &item
        {
            let target = if path.is_empty() { source } else { path };
            let own = target == source;
            let text = if own {
                current_text.to_string()
            } else {
                self.state
                    .texts()
                    .get(target)
                    .map(|text| text.to_string())
                    .ok_or_else(|| format!("Could not read {target}"))?
            };
            let cache = self.state.cache();
            let taken = cache
                .file(target)
                .map(|meta| meta.blocks.clone())
                .ok_or_else(|| format!("Could not find {target} in the Vault index"))?;
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos() as u64)
                .unwrap_or(0)
                ^ (*end as u64).wrapping_mul(0x9e37_79b9);
            let id = blocks::new_id(seed, &taken);
            let newline = if crlf && own {
                "\r\n"
            } else if text.contains("\r\n") {
                "\r\n"
            } else {
                "\n"
            };
            let insertion = blocks::plan_id(&text, *end, *placement, &id, newline)
                .ok_or_else(|| format!("No id can be written onto that block in {target}"))?;
            if own {
                own_note = Some(insertion);
            } else {
                self.state.write_block_id(target, &text, &insertion)?;
            }
            item = Item::Block {
                path: path.clone(),
                id: BlockId::Existing(id),
            };
        }

        let text = self.with_vault(source, |vault| {
            suggest::insert(trigger, &item, accept, vault, &options)
        });
        Ok(SuggestionInsertion { text, own_note })
    }
}
