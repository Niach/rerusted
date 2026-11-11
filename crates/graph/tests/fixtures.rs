use std::collections::BTreeSet;
use std::path::PathBuf;

use base::{ChildName, Fault};
use graph::{Dir, Manifest, PortPath, check, reach_of, sources_of};
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/graph")
}

fn text(name: &str) -> String {
    std::fs::read_to_string(dir().join(format!("{name}.json"))).unwrap()
}

fn load(name: &str) -> Manifest {
    Manifest::from_json(&text(name)).unwrap()
}

/// (code, pointer, wires) of each fault, in order.
fn summary(faults: &[Fault]) -> Vec<(String, String, Vec<String>)> {
    faults
        .iter()
        .map(|f| {
            let pointer = f
                .path
                .pointer
                .as_ref()
                .map(|p| p.as_str().to_owned())
                .unwrap_or_default();
            let wires = f.path.wires.iter().map(|p| p.as_str().to_owned()).collect();
            (f.code.as_str().to_owned(), pointer, wires)
        })
        .collect()
}

/// Faults as JSON without their ids.
fn without_ids(faults: &[Fault]) -> Value {
    let mut v = serde_json::to_value(faults).unwrap();
    for f in v.as_array_mut().unwrap() {
        f.as_object_mut().unwrap().remove("id");
    }
    v
}

/// (code, pointer, wires) of one expected fault.
type Expected = (&'static str, String, Vec<String>);

const FEED: &str = "/widget/body/compose/children/feed";

#[test]
fn examples_pass() {
    for name in ["git_activity", "twitter", "photocraft"] {
        let faults = check(&load(name));
        assert!(faults.is_empty(), "{name}: {:#?}", without_ids(&faults));
    }
}

#[test]
fn negatives_fail_with_their_rule_and_path() {
    let wire = |scope: &str, i: usize| format!("{scope}/body/compose/wires/{i}");
    let cases: [(&str, Vec<Expected>); 5] = [
        (
            "neg_c6_twitter_to_discord",
            vec![(
                "graph.label.not_accepted",
                "/widget/body/compose/children/discord/in/body".into(),
                vec![wire("/widget", 4)],
            )],
        ),
        (
            "neg_c3_unwired",
            vec![("graph.input.unwired", format!("{FEED}/in/posts"), vec![])],
        ),
        (
            "neg_c2_schema",
            vec![(
                "graph.wire.type_mismatch",
                wire(FEED, 0),
                vec![wire(FEED, 0)],
            )],
        ),
        (
            "neg_c5_cycle",
            vec![(
                "graph.cycle.no_state",
                wire(FEED, 1),
                vec![wire(FEED, 1), wire(FEED, 2)],
            )],
        ),
        (
            "neg_c1_outside_scope",
            vec![(
                "graph.wire.unknown_child",
                format!("{}/from", wire(FEED, 0)),
                vec![wire(FEED, 0)],
            )],
        ),
    ];
    for (name, expected) in cases {
        let got = summary(&check(&load(name)));
        let expected: Vec<_> = expected
            .into_iter()
            .map(|(c, p, w)| (c.to_owned(), p, w))
            .collect();
        assert_eq!(got, expected, "{name}");
    }
}

#[test]
fn c6_fault_names_source_sink_and_remedy() {
    let faults = check(&load("neg_c6_twitter_to_discord"));
    let v = serde_json::to_value(&faults[0]).unwrap();
    assert_eq!(v["fields"]["label"], "twitter");
    assert_eq!(v["fields"]["source"]["widget"][0], "twitter");
    assert_eq!(v["remedies"][0]["action"], "graph.wire.allow");
}

#[test]
fn allow_on_the_wire_fixes_c6() {
    let mut v: Value = serde_json::from_str(&text("neg_c6_twitter_to_discord")).unwrap();
    v["widget"]["body"]["compose"]["wires"][4]["allow"] = serde_json::json!(["twitter"]);
    let m: Manifest = serde_json::from_value(v).unwrap();
    assert!(check(&m).is_empty());
}

#[test]
fn checker_is_deterministic() {
    for entry in std::fs::read_dir(dir()).unwrap() {
        let path = entry.unwrap().path();
        let original = std::fs::read_to_string(&path).unwrap();
        // same document, keys re-ordered alphabetically
        let reordered =
            serde_json::to_string(&serde_json::from_str::<Value>(&original).unwrap()).unwrap();
        let a = check(&Manifest::from_json(&original).unwrap());
        let b = check(&Manifest::from_json(&original).unwrap());
        let c = check(&Manifest::from_json(&reordered).unwrap());
        assert_eq!(without_ids(&a), without_ids(&b), "{}", path.display());
        assert_eq!(without_ids(&a), without_ids(&c), "{}", path.display());
    }
}

#[test]
fn manifests_roundtrip() {
    for entry in std::fs::read_dir(dir()).unwrap() {
        let original: Value =
            serde_json::from_str(&std::fs::read_to_string(entry.unwrap().path()).unwrap()).unwrap();
        let m: Manifest = serde_json::from_value(original.clone()).unwrap();
        assert_eq!(serde_json::to_value(&m).unwrap(), original);
    }
}

#[test]
fn parse_errors_are_faults() {
    let f = Manifest::from_json(r#"{"id": "0193a1c2-7d4e-7b3e-9f10-2d4c6a8b0e11", "widget": {"body": {"compose": {"wires": [{"from": "a.b", "to": "c.in.d"}]}}}}"#).unwrap_err();
    assert_eq!(f.code.as_str(), "graph.manifest.parse");
    let f = Manifest::from_json(r#"{"id": "0193a1c2-7d4e-7b3e-9f10-2d4c6a8b0e11", "widget": {"in": {"x": {"kind": "data"}}, "body": {"compose": {}}}}"#).unwrap_err();
    assert_eq!(f.fields["category"], "data");
}

fn port(widget: &[&str], dir: Dir, port: &str) -> PortPath {
    PortPath::new(widget, dir, port).unwrap()
}

#[test]
fn queries_on_the_twitter_pipeline() {
    let m = load("twitter");
    assert_eq!(
        sources_of(&m, &port(&["feed", "list"], Dir::In, "rows")),
        // the egress's auth port also adds `twitter` (C0: every output-side egress port is labelled)
        BTreeSet::from([
            port(&["twitter"], Dir::In, "auth"),
            port(&["twitter"], Dir::Out, "posts")
        ])
    );
    assert_eq!(
        reach_of(&m, &[ChildName::new("feed").unwrap()]),
        BTreeSet::new()
    );
    assert_eq!(
        reach_of(&m, &[ChildName::new("twitter").unwrap()]),
        BTreeSet::from([
            port(&["feed", "list"], Dir::In, "rows"),
            port(&["feed", "log"], Dir::In, "value"),
            port(&["feed", "log"], Dir::Out, "value"),
            port(&["feed", "sort"], Dir::In, "items"),
            port(&["feed", "sort"], Dir::Out, "items"),
            port(&["secrets"], Dir::Out, "twitter"),
        ])
    );
}

#[test]
fn schema_is_current() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("manifest.schema.json");
    let generated = serde_json::to_string_pretty(&schemars::schema_for!(Manifest)).unwrap() + "\n";
    if std::env::var_os("UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &generated).unwrap();
    }
    let checked_in = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        checked_in == generated,
        "manifest.schema.json is stale; run UPDATE_SCHEMA=1 cargo test -p graph"
    );
}
