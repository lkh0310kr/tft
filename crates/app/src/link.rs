//! Link resolution decisions shared by every application host.

/// An internal wikilink after applying the vault's shared path resolution
/// rules. The parsed subpath borrows the original link body so hosts can use
/// the same heading/block target when scrolling or opening a new tab.
pub struct InternalLinkResolution<'a> {
    pub link: tft_markdown::LinkText<'a>,
    pub path: Option<String>,
}

/// Host independent interpretation of a rendered/editor link action.
pub enum LinkNavigation<'a> {
    Internal(InternalLinkResolution<'a>),
    External(&'a str),
    Tag(&'a str),
    Footnote(usize),
}

/// Classifies a link and resolves internal paths with the vault's canonical
/// resolver. Desktop and Web then apply the same parsed destination semantics.
pub fn resolve_link_navigation<'a>(
    resolver: &tft_cache::LinkResolver,
    target: &'a crate::reading::LinkTarget,
    source_path: &str,
) -> LinkNavigation<'a> {
    match target {
        crate::reading::LinkTarget::Internal(body) => {
            LinkNavigation::Internal(resolve_internal_link(resolver, body, source_path))
        }
        crate::reading::LinkTarget::External(url) => LinkNavigation::External(url),
        crate::reading::LinkTarget::Tag(tag) => LinkNavigation::Tag(tag),
        crate::reading::LinkTarget::Footnote(number) => LinkNavigation::Footnote(*number),
    }
}

pub fn resolve_internal_link<'a>(
    resolver: &tft_cache::LinkResolver,
    body: &'a str,
    source_path: &str,
) -> InternalLinkResolution<'a> {
    InternalLinkResolution {
        link: tft_markdown::parse_linktext(body),
        path: resolver.resolve(body, source_path).map(str::to_owned),
    }
}
