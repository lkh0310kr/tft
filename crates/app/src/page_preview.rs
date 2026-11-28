//! Shared page-preview sources and their vault-configured modifier behavior.

use tft_formats::config::PagePreviewConfig;

/// A UI location that can show a page preview. The variants map to
/// `page-preview.json`'s `hoverLinkSources` keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Reading,
    Editor,
    Files,
    TabHeader,
    Graph,
}

impl Source {
    /// Whether this source requires Cmd (macOS) or Ctrl (other platforms).
    pub fn requires_mod(self, config: &PagePreviewConfig) -> bool {
        match self {
            Self::Reading => config.preview.unwrap_or(false),
            Self::Editor => config.editor.unwrap_or(true),
            Self::Files => config.file_explorer.unwrap_or(true),
            Self::TabHeader => config.tab_header.unwrap_or(true),
            Self::Graph => config.graph.unwrap_or(true),
        }
    }
}
