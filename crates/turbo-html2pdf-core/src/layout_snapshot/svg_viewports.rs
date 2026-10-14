//! Explicit caller-owned outer SVG identities scope native viewport layout.
use super::{SnapshotError, SourceId, SourceIdentity, TextSnapshotError};
use crate::{StyledNode, Tag};
use std::collections::{BTreeMap, BTreeSet, HashSet};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SvgSnapshotError {
    #[error(transparent)]
    Text(#[from] TextSnapshotError),
    #[error("SVG layout scale must be finite and between 0.25 and 16")]
    InvalidLayoutScale,
    #[error("SVG viewport source {0:?} is not an identified SVG element")]
    InvalidViewportSource(SourceId),
    #[error("SVG viewport source {0:?} is declared more than once")]
    DuplicateViewport(SourceId),
    #[error("nested SVG viewport source {0:?} requires unsupported SVG layout")]
    NestedViewport(SourceId),
}
impl From<SnapshotError> for SvgSnapshotError {
    fn from(error: SnapshotError) -> Self {
        Self::Text(error.into())
    }
}

pub(super) struct Plan {
    pub(super) keys: HashSet<usize>,
    pub(super) layout_scale: f32,
    pub(super) unsupported: BTreeSet<SourceId>,
}

pub(super) fn prepare(
    styled: &[StyledNode],
    identities: &[SourceIdentity],
    viewports: &[SourceId],
    layout_scale: f32,
) -> Result<Plan, SvgSnapshotError> {
    validate_scale(layout_scale)?;
    let paths: BTreeMap<_, _> = identities.iter().map(|id| (id.id, &id.path)).collect();
    let (keys, roots) = declare_viewports(styled, &paths, viewports)?;
    check_nested(viewports, &paths, &roots)?;
    let unsupported = identities
        .iter()
        .filter(|id| under_root(&id.path, &roots))
        .map(|id| id.id)
        .collect();
    Ok(Plan {
        keys,
        unsupported,
        layout_scale,
    })
}

fn declare_viewports(
    styled: &[StyledNode],
    paths: &BTreeMap<SourceId, &Vec<usize>>,
    viewports: &[SourceId],
) -> Result<(HashSet<usize>, BTreeSet<Vec<usize>>), SvgSnapshotError> {
    let mut keys = HashSet::new();
    let mut roots = BTreeSet::new();
    for id in viewports {
        let (path, key) = identify_viewport(styled, paths, *id)?;
        if !keys.insert(key) {
            return Err(SvgSnapshotError::DuplicateViewport(*id));
        }
        roots.insert(path.clone());
    }
    Ok((keys, roots))
}

fn validate_scale(scale: f32) -> Result<(), SvgSnapshotError> {
    if scale.is_finite() && (0.25..=16.0).contains(&scale) {
        Ok(())
    } else {
        Err(SvgSnapshotError::InvalidLayoutScale)
    }
}

fn identify_viewport<'a>(
    styled: &[StyledNode],
    paths: &'a BTreeMap<SourceId, &Vec<usize>>,
    id: SourceId,
) -> Result<(&'a Vec<usize>, usize), SvgSnapshotError> {
    let path = paths
        .get(&id)
        .ok_or(SvgSnapshotError::InvalidViewportSource(id))?;
    let key = viewport_key(styled, path).ok_or(SvgSnapshotError::InvalidViewportSource(id))?;
    Ok((path, key))
}

fn check_nested(
    viewports: &[SourceId],
    paths: &BTreeMap<SourceId, &Vec<usize>>,
    roots: &BTreeSet<Vec<usize>>,
) -> Result<(), SvgSnapshotError> {
    for id in viewports {
        if under_root(paths[id], roots) {
            return Err(SvgSnapshotError::NestedViewport(*id));
        }
    }
    Ok(())
}

fn under_root(path: &[usize], roots: &BTreeSet<Vec<usize>>) -> bool {
    (0..path.len()).any(|length| roots.contains(&path[..length]))
}

fn viewport_key(mut nodes: &[StyledNode], path: &[usize]) -> Option<usize> {
    let (&last, parents) = path.split_last()?;
    for index in parents {
        let StyledNode::Element(element) = nodes.get(*index)? else {
            return None;
        };
        nodes = &element.children;
    }
    let StyledNode::Element(element) = nodes.get(last)? else {
        return None;
    };
    let Tag::Html(name) = &element.tag else {
        return None;
    };
    (name.rsplit(':').next() == Some("svg")).then_some(element as *const _ as usize)
}
