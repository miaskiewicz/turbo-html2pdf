use super::{
    SnapshotError, SnapshotFragment, SnapshotSource, SnapshotTextSource, SourceId, SourceIdentity,
    SourceKind,
};
use crate::layout::value::{display_of, Display};
use crate::{NodeId, StyledNode};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Debug, Clone)]
pub(crate) struct SourceRecord {
    pub(crate) id: Option<SourceId>,
    pub(crate) ancestors: Vec<SourceId>,
    pub(crate) inline_block_owners: Vec<SourceId>,
}

pub(crate) struct BuildSources {
    // Addresses are used only inside the one borrowed styled-tree construction.
    // They never escape as IDs; generated input labels cannot alias live nodes.
    nodes: HashMap<usize, SourceRecord>,
    pub(crate) svg_viewports: std::collections::HashSet<usize>,
    pub(crate) svg_layout_scale: f32,
    pub(crate) inline_borders: bool,
    pub(crate) records: HashMap<NodeId, SourceRecord>,
    pub(crate) text_sources: Vec<SnapshotTextSource>,
}

impl BuildSources {
    pub(crate) fn inline_block_owner(&mut self, block: NodeId, owner: NodeId) {
        let Some(owner) = self.records.get(&owner).and_then(|record| record.id) else {
            return;
        };
        if let Some(record) = self.records.get_mut(&block) {
            if !record.inline_block_owners.contains(&owner) {
                record.inline_block_owners.push(owner);
            }
        }
    }

    pub(crate) fn record(&mut self, node: NodeId, key: usize) {
        if let Some(source) = self.nodes.get(&key) {
            self.records.insert(node, source.clone());
        }
    }
}

pub(crate) fn prepare(
    styled: &[StyledNode],
    identities: &[SourceIdentity],
    capture_text: bool,
    native_html: bool,
) -> Result<(BuildSources, Vec<SnapshotSource>), SnapshotError> {
    let wanted = validate_identities(identities)?;
    let mut walker = Walker {
        wanted: &wanted,
        nodes: HashMap::new(),
        sources: Vec::new(),
        text_sources: Vec::new(),
        capture_text,
        native_html,
    };
    walker.visit(styled, &mut Vec::new(), &[], false);
    check_found(&wanted, &walker.sources)?;
    Ok((
        BuildSources {
            nodes: walker.nodes,
            svg_viewports: std::collections::HashSet::new(),
            svg_layout_scale: 1.0,
            inline_borders: false,
            records: HashMap::new(),
            text_sources: walker.text_sources,
        },
        walker.sources,
    ))
}

fn validate_identities(
    identities: &[SourceIdentity],
) -> Result<BTreeMap<Vec<usize>, SourceId>, SnapshotError> {
    let mut ids = BTreeSet::new();
    let mut paths = BTreeMap::new();
    for identity in identities {
        if !ids.insert(identity.id) {
            return Err(SnapshotError::DuplicateId(identity.id));
        }
        if paths.insert(identity.path.clone(), identity.id).is_some() {
            return Err(SnapshotError::DuplicatePath(identity.path.clone()));
        }
    }
    Ok(paths)
}

fn check_found(
    wanted: &BTreeMap<Vec<usize>, SourceId>,
    sources: &[SnapshotSource],
) -> Result<(), SnapshotError> {
    let found: BTreeSet<_> = sources.iter().map(|s| &s.path).collect();
    for path in wanted.keys() {
        if !found.contains(path) {
            return Err(SnapshotError::MissingPath(path.clone()));
        }
    }
    Ok(())
}

struct Walker<'a> {
    wanted: &'a BTreeMap<Vec<usize>, SourceId>,
    nodes: HashMap<usize, SourceRecord>,
    sources: Vec<SnapshotSource>,
    text_sources: Vec<SnapshotTextSource>,
    capture_text: bool,
    native_html: bool,
}

impl Walker<'_> {
    fn visit(
        &mut self,
        nodes: &[StyledNode],
        path: &mut Vec<usize>,
        ancestors: &[SourceId],
        excluded: bool,
    ) {
        for (i, node) in nodes.iter().enumerate() {
            path.push(i);
            self.node(node, path, ancestors, excluded);
            path.pop();
        }
    }

    fn node(
        &mut self,
        node: &StyledNode,
        path: &mut Vec<usize>,
        ancestors: &[SourceId],
        excluded: bool,
    ) {
        let excluded = excluded
            || matches!(node, StyledNode::Element(element) if display_of(&element.style) == Display::None
                || crate::layout::contents::suppresses_subtree(element));
        let id = self.wanted.get(path).copied();
        let (key, kind) = node_key(node);
        self.nodes.insert(
            key,
            SourceRecord {
                id,
                ancestors: ancestors.to_vec(),
                inline_block_owners: Vec::new(),
            },
        );
        if let Some(id) = id {
            self.capture_text_source(id, node);
            self.sources.push(SnapshotSource {
                id,
                path: path.clone(),
                kind,
                excluded_from_layout: excluded,
                fragment_indices: Vec::new(),
                descendant_fragment_indices: Vec::new(),
            });
        }
        self.children(node, path, ancestors, id, excluded);
    }

    fn capture_text_source(&mut self, id: SourceId, node: &StyledNode) {
        if !self.capture_text {
            return;
        }
        if let StyledNode::Text(text) = node {
            self.text_sources.push(SnapshotTextSource {
                source_id: id,
                text: text.clone(),
            });
        }
    }

    fn excludes_child_content(&self, element: &crate::StyledElement) -> bool {
        self.native_html
            && matches!(&element.tag, crate::Tag::Html(name) if name.eq_ignore_ascii_case("iframe"))
    }

    fn children(
        &mut self,
        node: &StyledNode,
        path: &mut Vec<usize>,
        ancestors: &[SourceId],
        id: Option<SourceId>,
        excluded: bool,
    ) {
        let StyledNode::Element(element) = node else {
            return;
        };
        let mut next = ancestors.to_vec();
        next.extend(id);
        // Fallback descendants remain in the DOM/source inventory, but a live
        // embedded viewport never renders them in its parent's formatting tree.
        let excluded = excluded || self.excludes_child_content(element);
        self.visit(&element.children, path, &next, excluded);
    }
}

fn node_key(node: &StyledNode) -> (usize, SourceKind) {
    match node {
        StyledNode::Element(element) => (
            element as *const crate::StyledElement as usize,
            SourceKind::Element,
        ),
        StyledNode::Text(_) => (node as *const StyledNode as usize, SourceKind::Text),
    }
}

pub(crate) fn index_fragments(
    sources: &mut [SnapshotSource],
    index: &BTreeMap<SourceId, usize>,
    fragments: &[SnapshotFragment],
    records: &HashMap<NodeId, SourceRecord>,
) {
    for fragment in fragments {
        if let Some(id) = fragment.source_id {
            sources[index[&id]].fragment_indices.push(fragment.index);
        }
        index_inline_blocks(fragment, records, sources, index);
        for id in &fragment.source_ancestors {
            sources[index[id]]
                .descendant_fragment_indices
                .push(fragment.index);
        }
    }
}

fn index_inline_blocks(
    fragment: &SnapshotFragment,
    records: &HashMap<NodeId, SourceRecord>,
    sources: &mut [SnapshotSource],
    index: &BTreeMap<SourceId, usize>,
) {
    if fragment.kind != super::GeometryKind::BorderBox {
        return;
    }
    if let Some(record) = records.get(&fragment.layout_node_id) {
        for owner in &record.inline_block_owners {
            sources[index[owner]].fragment_indices.push(fragment.index);
        }
    }
}
