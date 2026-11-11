//! Property tests on random compose trees, including invalid wiring.

use std::collections::{BTreeMap, BTreeSet};

use base::{ChildName, PortName, WidgetId};
use graph::{
    Body, Dir, Endpoint, Kind, Label, Manifest, Node, Port, PortKind, PortPath, Schema, Widget,
    Wire, check, labels_of, reach_from, reach_of, spec,
};
use proptest::prelude::*;

/// A small deterministic generator, driven by one proptest seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
    fn subset(&mut self, items: &[&str]) -> Vec<String> {
        items
            .iter()
            .filter(|_| self.chance(40))
            .map(|s| (*s).to_owned())
            .collect()
    }
}

const PORTS: [&str; 3] = ["a", "b", "c"];
const CHILDREN: [&str; 3] = ["p", "q", "r"];
const LABELS: [&str; 3] = ["l1", "l2", "l3"];
const KINDS: [Kind; 10] = [
    Kind::View,
    Kind::Expr,
    Kind::State,
    Kind::Egress,
    Kind::Secrets,
    Kind::Consent,
    Kind::Agent,
    Kind::Code,
    Kind::Process,
    Kind::System,
];

fn labels(names: Vec<String>) -> BTreeSet<Label> {
    names.into_iter().map(|l| Label::new(l).unwrap()).collect()
}

fn port(rng: &mut Rng) -> Port {
    let schema = |rng: &mut Rng| Schema((*rng.pick(&["s", "t"])).to_owned());
    let kind = match rng.below(3) {
        0 => PortKind::Data {
            schema: schema(rng),
        },
        1 => PortKind::Call {
            input: schema(rng),
            output: schema(rng),
        },
        _ => PortKind::View,
    };
    let mut p = Port::new(kind);
    p.optional = rng.chance(30);
    p.adds = labels(rng.subset(&LABELS));
    if rng.chance(40) {
        p.depends_on = Some(
            rng.subset(&PORTS)
                .into_iter()
                .map(|n| PortName::new(n).unwrap())
                .collect(),
        );
    }
    if rng.chance(40) {
        p.accepts = Some(labels(rng.subset(&LABELS)));
    }
    p
}

fn ports(rng: &mut Rng) -> BTreeMap<PortName, Port> {
    let mut out = BTreeMap::new();
    for n in PORTS {
        if rng.chance(60) {
            out.insert(PortName::new(n).unwrap(), port(rng));
        }
    }
    out
}

fn endpoint(rng: &mut Rng, children: &[&str]) -> Endpoint {
    let node = match rng.below(children.len() + 2) {
        0 => Node::This,
        1 => Node::Child(ChildName::new("x").unwrap()), // never a child: outside the scope
        i => Node::Child(ChildName::new(children[i - 2]).unwrap()),
    };
    let dir = if rng.chance(50) { Dir::In } else { Dir::Out };
    Endpoint {
        node,
        dir,
        port: PortName::new(*rng.pick(&PORTS)).unwrap(),
    }
}

fn widget(rng: &mut Rng, depth: usize, force_compose: bool) -> Widget {
    let body = if force_compose || (depth > 0 && rng.chance(50)) {
        let names: Vec<&str> = CHILDREN
            .iter()
            .copied()
            .filter(|_| rng.chance(70))
            .collect();
        let children = names
            .iter()
            .map(|n| {
                (
                    ChildName::new(*n).unwrap(),
                    widget(rng, depth.saturating_sub(1), false),
                )
            })
            .collect();
        let mut wires = Vec::new();
        for _ in 0..rng.below(10) {
            let allow = if rng.chance(30) {
                labels(rng.subset(&LABELS))
            } else {
                BTreeSet::new()
            };
            wires.push(Wire {
                from: endpoint(rng, &names),
                to: endpoint(rng, &names),
                allow,
            });
        }
        Body::Compose { children, wires }
    } else {
        Body::Primitive {
            kind: *rng.pick(&KINDS),
            config: serde_json::Value::Null,
        }
    };
    Widget {
        inputs: ports(rng),
        outputs: ports(rng),
        body,
    }
}

fn manifest(seed: u64, allow: bool) -> Manifest {
    let mut rng = Rng(seed | 1);
    let mut widget = widget(&mut rng, 3, true);
    if !allow {
        strip_allow(&mut widget);
    }
    Manifest {
        id: WidgetId::from(uuid_zero()),
        widget,
    }
}

fn uuid_zero() -> uuid::Uuid {
    uuid::Uuid::nil()
}

fn strip_allow(w: &mut Widget) {
    if let Body::Compose { children, wires } = &mut w.body {
        wires.iter_mut().for_each(|wire| wire.allow.clear());
        children.values_mut().for_each(strip_allow);
    }
}

fn walk<'a>(path: Vec<ChildName>, w: &'a Widget, out: &mut Vec<(Vec<ChildName>, &'a Widget)>) {
    out.push((path.clone(), w));
    if let Body::Compose { children, .. } = &w.body {
        for (name, child) in children {
            let mut p = path.clone();
            p.push(name.clone());
            walk(p, child, out);
        }
    }
}

fn widgets(m: &Manifest) -> Vec<(Vec<ChildName>, &Widget)> {
    let mut out = Vec::new();
    walk(Vec::new(), &m.widget, &mut out);
    out
}

/// The port a wire end names, if it exists and has the shape C1 requires.
fn resolve<'a>(scope: &'a Widget, e: &Endpoint, is_from: bool) -> Option<&'a Port> {
    let is_self = e.node == Node::This;
    if is_self != (e.dir == if is_from { Dir::In } else { Dir::Out }) {
        return None;
    }
    let w = match (&e.node, &scope.body) {
        (Node::This, _) => scope,
        (Node::Child(c), Body::Compose { children, .. }) => children.get(c)?,
        _ => return None,
    };
    w.ports(e.dir).get(&e.port)
}

fn is_call(p: &Port) -> bool {
    matches!(p.kind, PortKind::Call { .. })
}
fn is_data(p: &Port) -> bool {
    matches!(p.kind, PortKind::Data { .. })
}
fn input_side(dir: Dir, p: &Port) -> bool {
    is_call(p) || (dir == Dir::In && is_data(p))
}
fn output_side(dir: Dir, p: &Port) -> bool {
    is_call(p) || (dir == Dir::Out && is_data(p))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Wiring cannot create authority: whatever a widget's insides are wired to,
    /// all it reaches outside itself goes through its own boundary ports.
    #[test]
    fn wiring_cannot_create_authority(seed in any::<u64>()) {
        let m = manifest(seed, true);
        for (path, w) in widgets(&m) {
            if path.is_empty() {
                continue;
            }
            let outside = |p: &PortPath| !p.widget.starts_with(&path);
            let mut boundary: Vec<PortPath> = Vec::new();
            for dir in [Dir::In, Dir::Out] {
                for n in w.ports(dir).keys() {
                    boundary.push(PortPath { widget: path.clone(), dir, port: n.clone() });
                }
            }
            let via_boundary: BTreeSet<PortPath> = reach_from(&m, &boundary).into_iter().filter(outside).collect();
            let reached = reach_of(&m, &path);
            prop_assert!(reached.is_subset(&via_boundary), "{path:?}: {reached:?} not within {via_boundary:?}");
        }
    }

    /// Labels never shrink without an explicit allow, along wires and through primitives.
    #[test]
    fn labels_never_shrink_without_allow(seed in any::<u64>(), with_allow in any::<bool>()) {
        let m = manifest(seed, with_allow);
        let labels = labels_of(&m);
        let at = |p: &PortPath| labels.get(p).cloned().unwrap_or_default();
        for (path, w) in widgets(&m) {
            match &w.body {
                Body::Compose { wires, .. } => {
                    for wire in wires {
                        let (Some(f), Some(t)) = (resolve(w, &wire.from, true), resolve(w, &wire.to, false)) else { continue };
                        let (from, to) = (wire.from.at(&path), wire.to.at(&path));
                        let passed = |src: &PortPath| -> BTreeSet<Label> { at(src).difference(&wire.allow).cloned().collect() };
                        if is_call(f) || is_call(t) {
                            prop_assert!(passed(&from).is_subset(&at(&to)));
                            prop_assert!(passed(&to).is_subset(&at(&from)));
                        } else if is_data(f) || is_data(t) {
                            prop_assert!(passed(&from).is_subset(&at(&to)), "{from} -> {to}");
                        }
                    }
                }
                Body::Primitive { kind, .. } => {
                    let black_box = spec(*kind).black_box;
                    let inputs: Vec<(Dir, &PortName)> = [Dir::In, Dir::Out]
                        .into_iter()
                        .flat_map(|d| w.ports(d).iter().filter(move |(_, p)| input_side(d, p)).map(move |(n, _)| (d, n)))
                        .collect();
                    for dir in [Dir::In, Dir::Out] {
                        for (name, p) in w.ports(dir) {
                            let here = PortPath { widget: path.clone(), dir, port: name.clone() };
                            prop_assert!(p.adds.is_subset(&at(&here)));
                            let deps: Vec<(Dir, &PortName)> = match (&p.depends_on, black_box) {
                                (Some(names), false) => inputs.iter().filter(|(_, n)| names.contains(*n)).copied().collect(),
                                _ if output_side(dir, p) => inputs.clone(),
                                _ => Vec::new(),
                            };
                            for (d, n) in deps {
                                let src = PortPath { widget: path.clone(), dir: d, port: n.clone() };
                                if src != here {
                                    prop_assert!(at(&src).is_subset(&at(&here)), "{src} -> {here}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// The checker never panics and gives the same answer twice.
    #[test]
    fn checker_is_total_and_deterministic(seed in any::<u64>()) {
        let m = manifest(seed, true);
        let strip = |fs: Vec<base::Fault>| -> Vec<serde_json::Value> {
            fs.into_iter().map(|f| { let mut v = serde_json::to_value(f).unwrap(); v.as_object_mut().unwrap().remove("id"); v }).collect()
        };
        prop_assert_eq!(strip(check(&m)), strip(check(&m)));
        let text = serde_json::to_string(&m).unwrap();
        prop_assert_eq!(&Manifest::from_json(&text).unwrap(), &m);
    }
}
