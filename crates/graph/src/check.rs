//! The checker: rules C0 to C7. Returns every violation as a [`Fault`].

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use base::{ChildName, Class, Code, Fault, Pointer, Remedy};
use serde_json::{Value, json};

use crate::flow::{Flow, Unresolved, Via, dependency, input_side, output_side, resolve};
use crate::model::{
    Body, Dir, Endpoint, Manifest, PortKind, PortPath, Widget, port_pointer, walk, widget_pointer,
    wire_pointer,
};
use crate::registry::spec;

/// Checks a manifest. An empty list means it passes.
///
/// Faults are ordered by rule, then by pointer. The output is the same on
/// every run except for the fault ids.
pub fn check(m: &Manifest) -> Vec<Fault> {
    let mut out: Vec<(u8, Fault)> = Vec::new();
    let widgets = walk(&m.widget);
    for (path, w) in &widgets {
        well_formed(path, w, &mut out);
        if let Body::Compose { .. } = &w.body {
            scope(path, w, &mut out);
        }
    }
    let flow = Flow::build(m);
    cycles(&flow, &mut out);
    labels(&flow, &mut out);
    out.sort_by(|a, b| (a.0, &a.1.path.pointer).cmp(&(b.0, &b.1.path.pointer)));
    out.into_iter().map(|(_, f)| f).collect()
}

fn fault(rule: u8, code: &'static str, message: String) -> (u8, Fault) {
    (
        rule,
        Fault::new(Code::of(code), Class::Fixable, message).field("rule", format!("C{rule}")),
    )
}

fn names(path: &[ChildName]) -> Value {
    json!(path.iter().map(|c| c.as_str()).collect::<Vec<_>>())
}

fn port_at(path: &[ChildName], dir: Dir, port: &base::PortName) -> PortPath {
    PortPath {
        widget: path.to_vec(),
        dir,
        port: port.clone(),
    }
}

/// C0 (well-formed) and C7 (black boxes declare no dependencies).
fn well_formed(path: &[ChildName], w: &Widget, out: &mut Vec<(u8, Fault)>) {
    let kind = match &w.body {
        Body::Primitive { kind, .. } => *kind,
        Body::Compose { .. } => {
            for dir in [Dir::In, Dir::Out] {
                for (name, port) in w.ports(dir) {
                    if !port.adds.is_empty() || port.depends_on.is_some() || port.accepts.is_some()
                    {
                        let at = port_at(path, dir, name);
                        let (_, f) = fault(
                            0,
                            "graph.port.labels_on_compose",
                            format!("{at} declares labels, but only primitive ports can."),
                        );
                        out.push((0, f.field("port", &at).pointer(port_pointer(&at))));
                    }
                }
            }
            return;
        }
    };
    let s = spec(kind);
    if s.root_only && path.len() != 1 {
        let (r, f) = fault(
            0,
            "graph.primitive.root_only",
            format!(
                "A {} primitive can only be a direct child of the root.",
                kind.as_str()
            ),
        );
        out.push((
            r,
            f.field("kind", kind.as_str())
                .field("widget", names(path))
                .pointer(widget_pointer(path)),
        ));
    }
    for dir in [Dir::In, Dir::Out] {
        let allowed = if dir == Dir::In { s.inputs } else { s.outputs };
        for (name, port) in w.ports(dir) {
            let at = port_at(path, dir, name);
            let tag = port.tag();
            if !allowed.contains(&tag) {
                let (r, f) = fault(
                    0,
                    "graph.port.kind_not_allowed",
                    format!(
                        "A {} primitive cannot have a {tag} port on its {dir} side.",
                        kind.as_str()
                    ),
                );
                let allowed: Vec<Value> = allowed.iter().map(|t| json!(t)).collect();
                out.push((
                    r,
                    f.field("kind", kind.as_str())
                        .field("port", &at)
                        .field("port_kind", json!(tag))
                        .field("allowed", allowed)
                        .pointer(port_pointer(&at)),
                ));
            }
            if s.must_label && output_side(dir, tag) && port.adds.is_empty() {
                let (r, f) = fault(
                    0,
                    "graph.label.missing_adds",
                    format!(
                        "{at} sends data out of a {} primitive and must declare the labels it adds.",
                        kind.as_str()
                    ),
                );
                out.push((
                    r,
                    f.field("kind", kind.as_str())
                        .field("port", &at)
                        .pointer(port_pointer(&at)),
                ));
            }
            if s.sink && input_side(dir, tag) && port.accepts.is_none() {
                let (r, f) = fault(
                    0,
                    "graph.label.missing_accepts",
                    format!(
                        "{at} is a sink of a {} primitive and must declare the labels it accepts.",
                        kind.as_str()
                    ),
                );
                out.push((
                    r,
                    f.field("kind", kind.as_str())
                        .field("port", &at)
                        .pointer(port_pointer(&at)),
                ));
            }
            let Some(deps) = &port.depends_on else {
                continue;
            };
            if s.black_box {
                let (r, f) = fault(
                    7,
                    "graph.label.black_box_depends",
                    format!(
                        "{at} belongs to a {} primitive, a black box, and cannot narrow its dependencies.",
                        kind.as_str()
                    ),
                );
                out.push((
                    r,
                    f.field("kind", kind.as_str())
                        .field("port", &at)
                        .pointer(port_pointer(&at).push("depends_on")),
                ));
                continue;
            }
            for dep in deps {
                if dependency(w, dep).is_empty() {
                    let (r, f) = fault(
                        0,
                        "graph.port.unknown_dependency",
                        format!(
                            "{at} depends on {dep}, which is not an input-side port of the same widget."
                        ),
                    );
                    out.push((
                        r,
                        f.field("port", &at)
                            .field("dependency", dep.clone())
                            .pointer(port_pointer(&at).push("depends_on")),
                    ));
                }
            }
        }
    }
}

/// C1 to C4 for one compose scope.
fn scope(path: &[ChildName], w: &Widget, out: &mut Vec<(u8, Fault)>) {
    let Body::Compose { children, wires } = &w.body else {
        return;
    };
    let mut incoming: BTreeMap<&Endpoint, Vec<usize>> = BTreeMap::new();

    for (i, wire) in wires.iter().enumerate() {
        let ptr = wire_pointer(path, i);
        let from = resolve(w, &wire.from, true);
        let to = resolve(w, &wire.to, false);
        for (side, end, res) in [
            ("from", &wire.from, &from.map(|_| ())),
            ("to", &wire.to, &to.map(|_| ())),
        ] {
            let Err(why) = res else { continue };
            let (code, message) = match why {
                Unresolved::BadDirection => (
                    "graph.wire.bad_direction",
                    if side == "from" {
                        format!(
                            "A wire starts at {end}, but it can only start at self.in or a child's out."
                        )
                    } else {
                        format!(
                            "A wire ends at {end}, but it can only end at a child's in or self.out."
                        )
                    },
                ),
                Unresolved::UnknownChild => (
                    "graph.wire.unknown_child",
                    format!(
                        "A wire names {end}, but {} is not a child of this scope.",
                        end.node
                    ),
                ),
                Unresolved::UnknownPort => (
                    "graph.wire.unknown_port",
                    format!("A wire names {end}, but that widget has no such port."),
                ),
            };
            let (r, f) = fault(1, code, message);
            out.push((
                r,
                f.field("side", side)
                    .field("endpoint", format!("{end}"))
                    .field("scope", names(path))
                    .pointer(ptr.clone().push(side))
                    .wire(ptr.clone()),
            ));
        }
        if to.is_ok() {
            incoming.entry(&wire.to).or_default().push(i);
        }
        let (Ok(a), Ok(b)) = (from, to) else { continue };
        let mismatch = match (&a.kind, &b.kind) {
            (PortKind::Data { schema: x }, PortKind::Data { schema: y }) => (x != y).then(|| json!({ "from": { "schema": x }, "to": { "schema": y } })),
            (PortKind::Call { input: xi, output: xo }, PortKind::Call { input: yi, output: yo }) => {
                (xi != yi || xo != yo).then(|| json!({ "from": { "input": xi, "output": xo }, "to": { "input": yi, "output": yo } }))
            }
            (PortKind::View, PortKind::View) => None,
            _ => {
                let (r, f) = fault(2, "graph.wire.kind_mismatch", format!("A wire connects a {} port to a {} port.", a.tag(), b.tag()));
                out.push((r, f.field("from", format!("{}", wire.from)).field("to", format!("{}", wire.to)).field("from_kind", json!(a.tag())).field("to_kind", json!(b.tag())).pointer(ptr.clone()).wire(ptr)));
                continue;
            }
        };
        if let Some(types) = mismatch {
            let (r, f) = fault(
                2,
                "graph.wire.type_mismatch",
                format!(
                    "The wire from {} to {} connects ports of different types.",
                    wire.from, wire.to
                ),
            );
            out.push((
                r,
                f.field("from", format!("{}", wire.from))
                    .field("to", format!("{}", wire.to))
                    .field("types", types)
                    .pointer(ptr.clone())
                    .wire(ptr),
            ));
        }
    }

    // C3 and C4: every target that needs a wire has one, and none has two.
    let mut targets: Vec<(Endpoint, bool)> = Vec::new();
    for (name, child) in children {
        for (port, p) in &child.inputs {
            targets.push((
                Endpoint {
                    node: crate::Node::Child(name.clone()),
                    dir: Dir::In,
                    port: port.clone(),
                },
                p.optional,
            ));
        }
    }
    for (port, p) in &w.outputs {
        targets.push((
            Endpoint {
                node: crate::Node::This,
                dir: Dir::Out,
                port: port.clone(),
            },
            p.optional,
        ));
    }
    for (end, optional) in targets {
        let at = end.at(path);
        let count = incoming.get(&end).map_or(0, Vec::len);
        if count == 0 && !optional {
            let (r, f) = fault(
                3,
                "graph.input.unwired",
                format!("{end} is required but nothing is wired to it."),
            );
            let remedy = Remedy::new(Code::of("graph.wire.add"))
                .param("scope", widget_pointer(path))
                .param("to", format!("{end}"));
            out.push((
                r,
                f.field("port", &at)
                    .field("endpoint", format!("{end}"))
                    .pointer(port_pointer(&at))
                    .remedy(remedy),
            ));
        }
        if count > 1 {
            let (r, mut f) = fault(
                4,
                "graph.input.fan_in",
                format!("{end} has {count} incoming wires; at most one is allowed."),
            );
            for &i in &incoming[&end] {
                f = f.wire(wire_pointer(path, i));
            }
            out.push((
                r,
                f.field("port", &at)
                    .field("endpoint", format!("{end}"))
                    .field("count", count)
                    .pointer(port_pointer(&at)),
            ));
        }
    }
}

/// C5: no data cycle unless it passes through a `state` primitive.
fn cycles(flow: &Flow<'_>, out: &mut Vec<(u8, Fault)>) {
    let n = flow.nodes.len();
    let is_data = |i: usize| matches!(flow.nodes[i].port.kind, PortKind::Data { .. });
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in &flow.edges {
        if !is_data(e.from) || !is_data(e.to) {
            continue;
        }
        if e.via == Via::Internal
            && flow.nodes[e.to]
                .primitive
                .is_some_and(|k| spec(k).breaks_cycles)
        {
            continue;
        }
        adj[e.from].push(e.to);
    }
    for component in strongly_connected(&adj) {
        if component.len() < 2 {
            continue;
        }
        let members: BTreeSet<usize> = component.iter().copied().collect();
        let mut wires: BTreeSet<Pointer> = BTreeSet::new();
        for e in &flow.edges {
            if let Via::Wire { scope, index, .. } = &e.via
                && members.contains(&e.from)
                && members.contains(&e.to)
                && is_data(e.from)
                && is_data(e.to)
            {
                wires.insert(wire_pointer(scope, *index));
            }
        }
        let ports: Vec<Value> = members
            .iter()
            .map(|&i| Value::from(&flow.nodes[i].path))
            .collect();
        let (r, mut f) = fault(
            5,
            "graph.cycle.no_state",
            format!(
                "Data flows in a cycle through {} ports without passing a state primitive.",
                members.len()
            ),
        );
        f = f.field("ports", ports);
        if let Some(first) = wires.first() {
            f = f.pointer(first.clone());
        }
        for w in wires {
            f = f.wire(w);
        }
        out.push((r, f));
    }
}

/// Tarjan's algorithm, iterative. Components come out in a deterministic order.
fn strongly_connected(adj: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let n = adj.len();
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0; n];
    let mut on_stack = vec![false; n];
    let mut stack = Vec::new();
    let mut next = 0;
    let mut result = Vec::new();
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut work = vec![(root, 0usize)];
        while let Some(&mut (v, ref mut child)) = work.last_mut() {
            if *child == 0 && index[v] == usize::MAX {
                index[v] = next;
                low[v] = next;
                next += 1;
                stack.push(v);
                on_stack[v] = true;
            }
            if let Some(&w) = adj[v].get(*child) {
                *child += 1;
                if index[w] == usize::MAX {
                    work.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
                continue;
            }
            work.pop();
            if let Some(&(parent, _)) = work.last() {
                low[parent] = low[parent].min(low[v]);
            }
            if low[v] == index[v] {
                let mut component = Vec::new();
                while let Some(x) = stack.pop() {
                    on_stack[x] = false;
                    component.push(x);
                    if x == v {
                        break;
                    }
                }
                component.sort_unstable();
                result.push(component);
            }
        }
    }
    result
}

/// C6: a sink carries only the labels it accepts.
fn labels(flow: &Flow<'_>, out: &mut Vec<(u8, Fault)>) {
    let labels = flow.labels();
    for (sink, node) in flow.nodes.iter().enumerate() {
        let (Some(_), Some(accepts)) = (node.primitive, &node.port.accepts) else {
            continue;
        };
        for label in labels[sink].difference(accepts) {
            // Shortest path back to a port that adds the label, through edges that do not allow it.
            let mut came_from: BTreeMap<usize, usize> = BTreeMap::new();
            let mut queue = VecDeque::from([sink]);
            let mut seen = BTreeSet::from([sink]);
            let mut source = None;
            while let Some(v) = queue.pop_front() {
                if flow.nodes[v].primitive.is_some() && flow.nodes[v].port.adds.contains(label) {
                    source = Some(v);
                    break;
                }
                for &e in &flow.incoming[v] {
                    let edge = &flow.edges[e];
                    if !edge.allows(label)
                        && labels[edge.from].contains(label)
                        && seen.insert(edge.from)
                    {
                        came_from.insert(edge.from, e);
                        queue.push_back(edge.from);
                    }
                }
            }
            let mut wires: Vec<(usize, Pointer)> = Vec::new();
            let mut v = source.unwrap_or(sink);
            while let Some(&e) = came_from.get(&v) {
                if let Via::Wire { scope, index, .. } = &flow.edges[e].via {
                    wires.push((scope.len(), wire_pointer(scope, *index)));
                }
                v = flow.edges[e].to;
            }
            let accepted: Vec<&str> = accepts.iter().map(|l| l.as_str()).collect();
            let (r, mut f) = fault(
                6,
                "graph.label.not_accepted",
                if accepted.is_empty() {
                    format!(
                        "Data labelled {label} reaches {}, which accepts no labels.",
                        node.path
                    )
                } else {
                    format!(
                        "Data labelled {label} reaches {}, which accepts only {}.",
                        node.path,
                        accepted.join(", ")
                    )
                },
            );
            f = f
                .field("label", label.as_str())
                .field("accepts", json!(accepted))
                .field("sink", &node.path);
            if let Some(s) = source {
                f = f.field("source", &flow.nodes[s].path);
            }
            // The allow belongs on the outermost wire: the scope owner closest to the root.
            if let Some((_, wire)) = wires.iter().min_by_key(|(depth, _)| *depth) {
                f = f.remedy(
                    Remedy::new(Code::of("graph.wire.allow"))
                        .param("wire", wire.clone())
                        .param("labels", json!([label.as_str()])),
                );
            }
            for (_, w) in wires {
                f = f.wire(w);
            }
            out.push((r, f.pointer(port_pointer(&node.path))));
        }
    }
}
