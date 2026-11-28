//! Shared graph construction from the vault's canonical metadata and links.
use crate::VaultData;
use std::collections::{hash_map::Entry, HashMap, HashSet, VecDeque};
use tft_cache::{all_tags, MetadataCache};
use tft_formats::{
    config::{GraphColor, GraphConfig},
    FileKind,
};
use tft_search::{inputs, ExcludeFilters, Search, SearchFile};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphNodeKind {
    Note,
    Attachment,
    Unresolved,
    Tag,
}

#[derive(Debug, Clone)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub kind: GraphNodeKind,
    pub degree: usize,
    pub color: Option<GraphColor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalGraphOptions {
    pub jumps: u32,
    pub backlinks: bool,
    pub forelinks: bool,
    pub interlinks: bool,
}

#[derive(Debug, Clone, Default)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<(u32, u32)>,
    pub neighbors: Vec<Vec<usize>>,
}

/// Builds the filtered global or local graph. Both hosts pass their own
/// workspace state, while link resolution, filters, tags, and node ordering
/// stay identical here.
pub fn build_graph(
    vault: &VaultData,
    cache: &MetadataCache,
    excluded: &ExcludeFilters,
    config: &GraphConfig,
    local: Option<&str>,
    local_options: LocalGraphOptions,
) -> GraphData {
    let show_attachments = config.show_attachments.unwrap_or(false);
    let show_tags = config.show_tags.unwrap_or(false);
    let show_orphans = config.show_orphans.unwrap_or(true);
    let hide_unresolved = config.hide_unresolved.unwrap_or(false);
    let mut nodes = Vec::new();
    let mut index = HashMap::new();
    fn add(
        index: &mut HashMap<String, usize>,
        nodes: &mut Vec<GraphNode>,
        id: String,
        label: String,
        kind: GraphNodeKind,
    ) -> usize {
        if let Some(&ix) = index.get(&id) {
            return ix;
        }
        let ix = nodes.len();
        index.insert(id.clone(), ix);
        nodes.push(GraphNode {
            id,
            label,
            kind,
            degree: 0,
            color: None,
        });
        ix
    }
    let label_of = |path: &str| {
        let name = path.rsplit('/').next().unwrap_or(path);
        if FileKind::from_path(path) == FileKind::Markdown {
            name.trim_end_matches(".md").to_owned()
        } else {
            name.to_owned()
        }
    };
    let filter = config
        .search
        .as_deref()
        .filter(|q| !q.trim().is_empty())
        .and_then(|q| Search::new(q).ok());
    let passes = |path: &str| {
        if excluded.is_excluded(path) {
            return false;
        }
        let Some(search) = &filter else {
            return true;
        };
        let (Some(meta), Some(node)) = (cache.file(path), vault.lookup(path).ok()) else {
            return true;
        };
        let Some(text) = vault.text(node) else {
            return true;
        };
        let indexed = inputs(meta, text);
        search
            .matches(&SearchFile {
                path,
                content: text,
                tags: &indexed.tags,
                frontmatter: meta.frontmatter.as_ref(),
                blocks: &indexed.blocks,
                sections: &indexed.sections,
                tasks: &indexed.tasks,
            })
            .is_some()
    };
    for (path, _) in cache.files() {
        if passes(path) {
            add(
                &mut index,
                &mut nodes,
                path.to_owned(),
                label_of(path),
                GraphNodeKind::Note,
            );
        }
    }
    if show_attachments {
        for file in vault.files() {
            if file.kind != FileKind::Markdown
                && file.kind != FileKind::Other
                && !excluded.is_excluded(&file.path)
            {
                add(
                    &mut index,
                    &mut nodes,
                    file.path.clone(),
                    label_of(&file.path),
                    GraphNodeKind::Attachment,
                );
            }
        }
    }
    if let Some(center) = local {
        let kind = if FileKind::from_path(center) == FileKind::Markdown {
            GraphNodeKind::Note
        } else {
            GraphNodeKind::Attachment
        };
        add(
            &mut index,
            &mut nodes,
            center.to_owned(),
            label_of(center),
            kind,
        );
    }
    let mut edge_set = HashSet::new();
    let note_paths = cache
        .files()
        .map(|(path, _)| path.to_owned())
        .collect::<Vec<_>>();
    for path in &note_paths {
        let Some(&from) = index.get(path) else {
            continue;
        };
        if let Some(targets) = cache.resolved_links(path) {
            for target in targets.keys() {
                let kind = FileKind::from_path(target);
                let is_center = local == Some(target.as_str());
                if kind != FileKind::Markdown && !show_attachments && !is_center {
                    continue;
                }
                if excluded.is_excluded(target) && !is_center {
                    continue;
                }
                let to = match index.get(target) {
                    Some(&ix) => ix,
                    None if kind == FileKind::Markdown => continue,
                    None => add(
                        &mut index,
                        &mut nodes,
                        target.clone(),
                        label_of(target),
                        GraphNodeKind::Attachment,
                    ),
                };
                if from != to {
                    edge_set.insert((from as u32, to as u32));
                }
            }
        }
        if !hide_unresolved {
            if let Some(targets) = cache.unresolved_links(path) {
                for target in targets.keys() {
                    if !target.is_empty() {
                        let to = add(
                            &mut index,
                            &mut nodes,
                            format!("unresolved:{}", target.to_lowercase()),
                            target.clone(),
                            GraphNodeKind::Unresolved,
                        );
                        edge_set.insert((from as u32, to as u32));
                    }
                }
            }
        }
        if show_tags {
            if let Some(meta) = cache.file(path) {
                for tag in all_tags(meta) {
                    let to = add(
                        &mut index,
                        &mut nodes,
                        format!("tag:{}", tag.to_lowercase()),
                        format!("#{tag}"),
                        GraphNodeKind::Tag,
                    );
                    edge_set.insert((from as u32, to as u32));
                }
            }
        }
    }
    let groups = config
        .color_groups
        .iter()
        .filter(|g| !g.query.trim().is_empty())
        .filter_map(|g| Some((Search::new(&g.query).ok()?, g.color.clone())))
        .collect::<Vec<_>>();
    if !groups.is_empty() {
        for node in nodes.iter_mut().filter(|n| n.kind == GraphNodeKind::Note) {
            let (Some(meta), Some(id)) = (cache.file(&node.id), vault.lookup(&node.id).ok()) else {
                continue;
            };
            let Some(text) = vault.text(id) else {
                continue;
            };
            let indexed = inputs(meta, text);
            let file = SearchFile {
                path: &node.id,
                content: text,
                tags: &indexed.tags,
                frontmatter: meta.frontmatter.as_ref(),
                blocks: &indexed.blocks,
                sections: &indexed.sections,
                tasks: &indexed.tasks,
            };
            node.color = groups
                .iter()
                .find(|(search, _)| search.matches(&file).is_some())
                .map(|(_, color)| color.clone());
        }
    }
    let mut edges = edge_set.into_iter().collect::<Vec<_>>();
    edges.sort();
    if let Some(center) = local {
        let Some(&center_index) = index.get(center) else {
            return GraphData::default();
        };
        let (keep, local_edges) = local_subgraph(&edges, center_index, local_options);
        return finish_graph(nodes, local_edges, Some(keep), true);
    }
    finish_graph(nodes, edges, None, show_orphans)
}

fn local_subgraph(
    edges: &[(u32, u32)],
    center: usize,
    options: LocalGraphOptions,
) -> (HashSet<usize>, Vec<(u32, u32)>) {
    let mut outgoing: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut incoming: HashMap<usize, Vec<usize>> = HashMap::new();
    for &(a, b) in edges {
        outgoing.entry(a as usize).or_default().push(b as usize);
        incoming.entry(b as usize).or_default().push(a as usize);
    }
    let mut depth = HashMap::new();
    let mut queue = VecDeque::new();
    depth.insert(center, 0usize);
    queue.push_back((center, 0usize));
    while let Some((node, d)) = queue.pop_front() {
        if d >= options.jumps as usize {
            continue;
        }
        for &other in options
            .forelinks
            .then(|| outgoing.get(&node))
            .flatten()
            .into_iter()
            .chain(
                options
                    .backlinks
                    .then(|| incoming.get(&node))
                    .flatten()
                    .into_iter(),
            )
            .flatten()
        {
            if let Entry::Vacant(slot) = depth.entry(other) {
                slot.insert(d + 1);
                queue.push_back((other, d + 1));
            }
        }
    }
    let kept_edges = edges
        .iter()
        .copied()
        .filter(|&(a, b)| {
            let (Some(&da), Some(&db)) = (depth.get(&(a as usize)), depth.get(&(b as usize)))
            else {
                return false;
            };
            options.interlinks || da.abs_diff(db) == 1
        })
        .collect();
    (depth.into_keys().collect(), kept_edges)
}

fn finish_graph(
    nodes: Vec<GraphNode>,
    edges: Vec<(u32, u32)>,
    keep: Option<HashSet<usize>>,
    show_orphans: bool,
) -> GraphData {
    let mut degree = vec![0usize; nodes.len()];
    for &(a, b) in &edges {
        degree[a as usize] += 1;
        degree[b as usize] += 1;
    }
    let mut remap = vec![usize::MAX; nodes.len()];
    let mut out_nodes = Vec::new();
    for (i, mut node) in nodes.into_iter().enumerate() {
        let kept = keep.as_ref().is_none_or(|set| set.contains(&i));
        if !kept || (degree[i] == 0 && !show_orphans) {
            continue;
        }
        node.degree = degree[i];
        remap[i] = out_nodes.len();
        out_nodes.push(node);
    }
    let mut out_edges = Vec::new();
    let mut neighbors = vec![Vec::new(); out_nodes.len()];
    for (a, b) in edges {
        let (a, b) = (remap[a as usize], remap[b as usize]);
        if a == usize::MAX || b == usize::MAX {
            continue;
        }
        out_edges.push((a as u32, b as u32));
        neighbors[a].push(b);
        neighbors[b].push(a);
    }
    for (i, node) in out_nodes.iter_mut().enumerate() {
        node.degree = neighbors[i].len();
    }
    GraphData {
        nodes: out_nodes,
        edges: out_edges,
        neighbors,
    }
}
