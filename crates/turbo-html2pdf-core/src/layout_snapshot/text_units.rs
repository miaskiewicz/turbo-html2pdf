use super::{sources::SourceRecord, SnapshotError, SnapshotTextSource, SourceId, SourceTextUnits};
use crate::NodeId;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

pub(crate) type NativeUnitMap = HashMap<NodeId, Arc<[u16]>>;

pub(super) fn prepare(
    sources: &[SnapshotTextSource],
    inputs: &[SourceTextUnits<'_>],
) -> Result<BTreeMap<SourceId, Arc<[u16]>>, SnapshotError> {
    let mut output: BTreeMap<_, _> = sources
        .iter()
        .map(|source| {
            (
                source.source_id,
                Arc::from(source.text.encode_utf16().collect::<Vec<_>>()),
            )
        })
        .collect();
    let projected: BTreeMap<_, _> = sources
        .iter()
        .map(|source| (source.source_id, source.text.as_str()))
        .collect();
    let mut seen = std::collections::BTreeSet::new();
    let mut remaining: usize = 2_000_000;
    for input in inputs {
        remaining = remaining
            .checked_sub(input.units.len())
            .ok_or(SnapshotError::OriginalTextBudget)?;
        validate_source(&projected, &mut seen, input)?;
        output.insert(input.source_id, Arc::from(input.units));
    }
    Ok(output)
}

fn validate_source(
    projected: &BTreeMap<SourceId, &str>,
    seen: &mut std::collections::BTreeSet<SourceId>,
    input: &SourceTextUnits<'_>,
) -> Result<(), SnapshotError> {
    let scalar = projected
        .get(&input.source_id)
        .ok_or(SnapshotError::InvalidOriginalText(input.source_id))?;
    if !seen.insert(input.source_id) || String::from_utf16_lossy(input.units) != *scalar {
        return Err(SnapshotError::InvalidOriginalText(input.source_id));
    }
    Ok(())
}

pub(super) fn native_nodes(
    records: &HashMap<NodeId, SourceRecord>,
    units: &BTreeMap<SourceId, Arc<[u16]>>,
    enabled: bool,
) -> Option<Arc<NativeUnitMap>> {
    enabled.then(|| {
        Arc::new(
            records
                .iter()
                .filter_map(|(node, record)| {
                    record
                        .id
                        .and_then(|id| units.get(&id))
                        .map(|units| (*node, units.clone()))
                })
                .collect(),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sources() -> [SnapshotTextSource; 1] {
        [SnapshotTextSource {
            source_id: SourceId(7),
            text: "a\u{fffd}b".into(),
        }]
    }
    #[test]
    fn original_units_preserve_lones_and_scalar_defaults_without_aliasing_sources() {
        let sources = sources();
        let original = [0x61, 0xd800, 0x62];
        let prepared = prepare(
            &sources,
            &[SourceTextUnits {
                source_id: SourceId(7),
                units: &original,
            }],
        )
        .unwrap();
        assert_eq!(prepared[&SourceId(7)].as_ref(), original);
        assert_eq!(
            prepare(&sources, &[]).unwrap()[&SourceId(7)].as_ref(),
            [0x61, 0xfffd, 0x62]
        );
        let records = HashMap::from([
            (
                NodeId(1),
                SourceRecord {
                    inline_block_owners: Vec::new(),
                    id: Some(SourceId(7)),
                    ancestors: vec![],
                },
            ),
            (
                NodeId(2),
                SourceRecord {
                    inline_block_owners: Vec::new(),
                    id: None,
                    ancestors: vec![],
                },
            ),
            (
                NodeId(3),
                SourceRecord {
                    inline_block_owners: Vec::new(),
                    id: Some(SourceId(8)),
                    ancestors: vec![],
                },
            ),
        ]);
        assert!(native_nodes(&records, &prepared, false).is_none());
        let nodes = native_nodes(&records, &prepared, true).unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[&NodeId(1)].as_ref(), original);
    }
    #[test]
    fn unknown_duplicate_mismatched_and_excessive_original_inputs_fail_closed() {
        let sources = sources();
        let valid = SourceTextUnits {
            source_id: SourceId(7),
            units: &[0x61, 0xd800, 0x62],
        };
        assert_eq!(
            prepare(&sources, &[valid, valid]),
            Err(SnapshotError::InvalidOriginalText(SourceId(7)))
        );
        assert_eq!(
            prepare(
                &sources,
                &[SourceTextUnits {
                    source_id: SourceId(99),
                    ..valid
                }]
            ),
            Err(SnapshotError::InvalidOriginalText(SourceId(99)))
        );
        assert_eq!(
            prepare(
                &sources,
                &[SourceTextUnits {
                    source_id: SourceId(7),
                    units: &[0x61]
                }]
            ),
            Err(SnapshotError::InvalidOriginalText(SourceId(7)))
        );
        let excessive = vec![0x61; 2_000_001];
        assert_eq!(
            prepare(
                &sources,
                &[SourceTextUnits {
                    source_id: SourceId(7),
                    units: &excessive
                }]
            ),
            Err(SnapshotError::OriginalTextBudget)
        );
    }
}
