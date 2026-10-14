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
}

pub(crate) struct BuildSources {
    // Addresses are used only inside the one borrowed styled-tree construction.
    // They never escape as IDs; generated input labels cannot alias live nodes.
    nodes: HashMap<usize, SourceRecord>,
    pub(crate) svg_viewports: std::collections::HashSet<usize>,
    pub(crate) svg_layout_scale: f32,
    pub(crate) records: HashMap<NodeId, SourceRecord>,
    pub(crate) text_sources: Vec<SnapshotTextSource>,
}

impl BuildSources {
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
) -> Result<(BuildSources, Vec<SnapshotSource>), SnapshotError> {
    let wanted = validate_identities(identities)?;
    let mut walker = Walker {
        wanted: &wanted,
        nodes: HashMap::new(),
        sources: Vec::new(),
        text_sources: Vec::new(),
        capture_text,
    };
    walker.visit(styled, &mut Vec::new(), &[], false);
    check_found(&wanted, &walker.sources)?;
    Ok((
        BuildSources {
            nodes: walker.nodes,
            svg_viewports: std::collections::HashSet::new(),
            svg_layout_scale: 1.0,
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
            || matches!(node, StyledNode::Element(element) if display_of(&element.style) == Display::None);
        let id = self.wanted.get(path).copied();
        let (key, kind) = node_key(node);
        self.nodes.insert(
            key,
            SourceRecord {
                id,
                ancestors: ancestors.to_vec(),
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
) {
    for fragment in fragments {
        if let Some(id) = fragment.source_id {
            sources[index[&id]].fragment_indices.push(fragment.index);
        }
        for id in &fragment.source_ancestors {
            sources[index[id]]
                .descendant_fragment_indices
                .push(fragment.index);
        }
    }
}
