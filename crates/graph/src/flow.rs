//! The flow graph: every port of every widget is a node; wires and the
//! dependencies inside primitives are edges. Labels, cycles and the queries
//! are all computed on it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use base::{ChildName, PortName};

use crate::model::{
    Body, Dir, Endpoint, Kind, Label, Manifest, Node, Port, PortKindTag, PortPath, Widget, Wire,
    walk,
};
use crate::registry::spec;

/// Why an endpoint does not resolve (C1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Unresolved {
    BadDirection,
    UnknownChild,
    UnknownPort,
}

/// The port an endpoint names inside `scope`, if it exists and has the right shape.
pub(crate) fn resolve<'a>(
    scope: &'a Widget,
    e: &Endpoint,
    is_from: bool,
) -> Result<&'a Port, Unresolved> {
    let is_self = e.node == Node::This;
    // from: self.in or child.out; to: child.in or self.out
    let shape_ok = if is_from {
        is_self == (e.dir == Dir::In)
    } else {
        is_self == (e.dir == Dir::Out)
    };
    if !shape_ok {
        return Err(Unresolved::BadDirection);
    }
    let widget = match &e.node {
        Node::This => scope,
        Node::Child(c) => match &scope.body {
            Body::Compose { children, .. } => children.get(c).ok_or(Unresolved::UnknownChild)?,
            Body::Primitive { .. } => return Err(Unresolved::UnknownChild),
        },
    };
    widget
        .ports(e.dir)
        .get(&e.port)
        .ok_or(Unresolved::UnknownPort)
}

/// Input-side: what flows into a primitive through this port.
pub(crate) fn input_side(dir: Dir, tag: PortKindTag) -> bool {
    matches!(
        (dir, tag),
        (Dir::In, PortKindTag::Data) | (_, PortKindTag::Call)
    )
}

/// Output-side: what flows out of a primitive through this port.
pub(crate) fn output_side(dir: Dir, tag: PortKindTag) -> bool {
    matches!(
        (dir, tag),
        (Dir::Out, PortKindTag::Data) | (_, PortKindTag::Call)
    )
}

/// The input-side ports a `depends_on` name stands for: every one with that name, on either side.
pub(crate) fn dependency(w: &Widget, name: &PortName) -> Vec<Dir> {
    [Dir::In, Dir::Out]
        .into_iter()
        .filter(|&dir| {
            w.ports(dir)
                .get(name)
                .is_some_and(|p| input_side(dir, p.tag()))
        })
        .collect()
}

pub(crate) struct NodeInfo<'a> {
    pub path: PortPath,
    pub port: &'a Port,
    /// The primitive kind if the port belongs to a primitive.
    pub primitive: Option<Kind>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Via<'a> {
    /// Wire `index` of the scope at `scope`.
    Wire {
        scope: Vec<ChildName>,
        index: usize,
        allow: &'a BTreeSet<Label>,
    },
    /// A dependency inside a primitive.
    Internal,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Edge<'a> {
    pub from: usize,
    pub to: usize,
    pub via: Via<'a>,
}

pub(crate) struct Flow<'a> {
    pub nodes: Vec<NodeInfo<'a>>,
    pub index: BTreeMap<PortPath, usize>,
    pub edges: Vec<Edge<'a>>,
    /// Edge indices leaving each node.
    pub outgoing: Vec<Vec<usize>>,
    /// Edge indices entering each node.
    pub incoming: Vec<Vec<usize>>,
}

impl<'a> Flow<'a> {
    pub fn build(m: &'a Manifest) -> Flow<'a> {
        let widgets = walk(&m.widget);

        let mut found: Vec<NodeInfo<'a>> = Vec::new();
        for (path, w) in &widgets {
            let primitive = match &w.body {
                Body::Primitive { kind, .. } => Some(*kind),
                Body::Compose { .. } => None,
            };
            for dir in [Dir::In, Dir::Out] {
                for (name, port) in w.ports(dir) {
                    let path = PortPath {
                        widget: path.clone(),
                        dir,
                        port: name.clone(),
                    };
                    found.push(NodeInfo {
                        path,
                        port,
                        primitive,
                    });
                }
            }
        }
        found.sort_by(|a, b| a.path.cmp(&b.path));
        let index: BTreeMap<PortPath, usize> = found
            .iter()
            .enumerate()
            .map(|(i, n)| (n.path.clone(), i))
            .collect();

        let mut edges = Vec::new();
        for (path, w) in &widgets {
            match &w.body {
                Body::Compose { wires, .. } => {
                    for (i, wire) in wires.iter().enumerate() {
                        wire_edges(&index, path, w, i, wire, &mut edges);
                    }
                }
                Body::Primitive { kind, .. } => internal_edges(&index, path, w, *kind, &mut edges),
            }
        }
        edges.sort();
        edges.dedup();

        let mut outgoing = vec![Vec::new(); found.len()];
        let mut incoming = vec![Vec::new(); found.len()];
        for (i, e) in edges.iter().enumerate() {
            outgoing[e.from].push(i);
            incoming[e.to].push(i);
        }
        Flow {
            nodes: found,
            index,
            edges,
            outgoing,
            incoming,
        }
    }

    /// The labels each port carries: the smallest sets closed under the edges.
    pub fn labels(&self) -> Vec<BTreeSet<Label>> {
        let mut labels: Vec<BTreeSet<Label>> = self
            .nodes
            .iter()
            .map(|n| {
                if n.primitive.is_some() {
                    n.port.adds.clone()
                } else {
                    BTreeSet::new()
                }
            })
            .collect();
        let mut queue: VecDeque<usize> = (0..self.nodes.len()).collect();
        while let Some(n) = queue.pop_front() {
            for &e in &self.outgoing[n] {
                let edge = &self.edges[e];
                let pass: Vec<Label> = labels[n]
                    .iter()
                    .filter(|l| !edge.allows(l))
                    .cloned()
                    .collect();
                let target = &mut labels[edge.to];
                let before = target.len();
                target.extend(pass);
                if target.len() != before {
                    queue.push_back(edge.to);
                }
            }
        }
        labels
    }

    /// Every node reachable from `start` along edges, ignoring `allow`.
    pub fn reach(&self, start: impl IntoIterator<Item = usize>, forward: bool) -> BTreeSet<usize> {
        let mut seen: BTreeSet<usize> = BTreeSet::new();
        let mut queue: VecDeque<usize> = start.into_iter().collect();
        seen.extend(queue.iter().copied());
        while let Some(n) = queue.pop_front() {
            let list = if forward {
                &self.outgoing[n]
            } else {
                &self.incoming[n]
            };
            for &e in list {
                let next = if forward {
                    self.edges[e].to
                } else {
                    self.edges[e].from
                };
                if seen.insert(next) {
                    queue.push_back(next);
                }
            }
        }
        seen
    }
}

impl Edge<'_> {
    pub fn allows(&self, label: &Label) -> bool {
        matches!(&self.via, Via::Wire { allow, .. } if allow.contains(label))
    }
}

fn wire_edges<'a>(
    index: &BTreeMap<PortPath, usize>,
    scope: &[ChildName],
    w: &Widget,
    i: usize,
    wire: &'a Wire,
    edges: &mut Vec<Edge<'a>>,
) {
    // C1: a wire whose ends do not resolve creates no edge.
    let (Ok(from_port), Ok(to_port)) = (resolve(w, &wire.from, true), resolve(w, &wire.to, false))
    else {
        return;
    };
    let (Some(&from), Some(&to)) = (
        index.get(&wire.from.at(scope)),
        index.get(&wire.to.at(scope)),
    ) else {
        return;
    };
    let tags = [from_port.tag(), to_port.tag()];
    let via = || Via::Wire {
        scope: scope.to_vec(),
        index: i,
        allow: &wire.allow,
    };
    // a call wire passes labels both ways, a data wire forward, a view wire none
    if tags.contains(&PortKindTag::Call) {
        edges.push(Edge {
            from,
            to,
            via: via(),
        });
        edges.push(Edge {
            from: to,
            to: from,
            via: via(),
        });
    } else if tags.contains(&PortKindTag::Data) {
        edges.push(Edge {
            from,
            to,
            via: via(),
        });
    }
}

fn internal_edges(
    index: &BTreeMap<PortPath, usize>,
    path: &[ChildName],
    w: &Widget,
    kind: Kind,
    edges: &mut Vec<Edge<'_>>,
) {
    let black_box = spec(kind).black_box;
    let node = |dir: Dir, name: &PortName| {
        index[&PortPath {
            widget: path.to_vec(),
            dir,
            port: name.clone(),
        }]
    };
    let all_inputs: Vec<(Dir, &PortName)> = [Dir::In, Dir::Out]
        .into_iter()
        .flat_map(|dir| {
            w.ports(dir)
                .iter()
                .filter(move |(_, p)| input_side(dir, p.tag()))
                .map(move |(n, _)| (dir, n))
        })
        .collect();

    for dir in [Dir::In, Dir::Out] {
        for (name, port) in w.ports(dir) {
            let deps: Vec<(Dir, &PortName)> = match (&port.depends_on, black_box) {
                (Some(names), false) => names
                    .iter()
                    .flat_map(|n| dependency(w, n).into_iter().map(move |d| (d, n)))
                    .collect(),
                _ if output_side(dir, port.tag()) => all_inputs.clone(),
                _ => Vec::new(),
            };
            let to = node(dir, name);
            for (d, n) in deps {
                let from = node(d, n);
                if from != to {
                    edges.push(Edge {
                        from,
                        to,
                        via: Via::Internal,
                    });
                }
            }
        }
    }
}
