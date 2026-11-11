//! Queries for the privacy dashboard. They show real flows: `allow` is ignored.

use std::collections::{BTreeMap, BTreeSet};

use base::ChildName;

use crate::flow::Flow;
use crate::model::{Label, Manifest, PortPath};

/// The primitive ports that add labels and whose flow reaches `sink`.
pub fn sources_of(m: &Manifest, sink: &PortPath) -> BTreeSet<PortPath> {
    let flow = Flow::build(m);
    let Some(&start) = flow.index.get(sink) else {
        return BTreeSet::new();
    };
    flow.reach([start], false)
        .into_iter()
        .filter(|&i| i != start)
        .map(|i| &flow.nodes[i])
        .filter(|n| n.primitive.is_some() && !n.port.adds.is_empty())
        .map(|n| n.path.clone())
        .collect()
}

/// The primitive ports outside `widget` reachable from any port inside it.
pub fn reach_of(m: &Manifest, widget: &[ChildName]) -> BTreeSet<PortPath> {
    let flow = Flow::build(m);
    let inside = |p: &PortPath| p.widget.starts_with(widget);
    let start: Vec<usize> = flow
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| inside(&n.path))
        .map(|(i, _)| i)
        .collect();
    flow.reach(start, true)
        .into_iter()
        .map(|i| &flow.nodes[i])
        .filter(|n| n.primitive.is_some() && !inside(&n.path))
        .map(|n| n.path.clone())
        .collect()
}

/// The primitive ports reachable from the given ports (the ports themselves included).
pub fn reach_from(m: &Manifest, ports: &[PortPath]) -> BTreeSet<PortPath> {
    let flow = Flow::build(m);
    let start: Vec<usize> = ports
        .iter()
        .filter_map(|p| flow.index.get(p).copied())
        .collect();
    flow.reach(start, true)
        .into_iter()
        .map(|i| &flow.nodes[i])
        .filter(|n| n.primitive.is_some())
        .map(|n| n.path.clone())
        .collect()
}

/// The labels every port carries, with `allow` applied (the C6 view).
pub fn labels_of(m: &Manifest) -> BTreeMap<PortPath, BTreeSet<Label>> {
    let flow = Flow::build(m);
    flow.labels()
        .into_iter()
        .zip(&flow.nodes)
        .map(|(l, n)| (n.path.clone(), l))
        .collect()
}
