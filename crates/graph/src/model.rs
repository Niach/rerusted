//! The widget model and its JSON form (the manifest).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use base::{ChildName, Class, Code, Fault, Pointer, PortName, WidgetId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A widget manifest: the root widget of a tree plus its id.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub id: WidgetId,
    pub widget: Widget,
}

impl Manifest {
    /// Reads a manifest. A parse error is one `graph.manifest.parse` fault.
    // Fault is the project's one error type and carries full context by design.
    #[allow(clippy::result_large_err)]
    pub fn from_json(text: &str) -> Result<Manifest, Fault> {
        serde_json::from_str(text).map_err(|e| {
            Fault::new(
                Code::of("graph.manifest.parse"),
                Class::Fixable,
                "The manifest is not valid JSON of the manifest format.",
            )
            .field("line", e.line())
            .field("column", e.column())
            .field("category", category(&e))
            .pointer(Pointer::root())
        })
    }
}

fn category(e: &serde_json::Error) -> &'static str {
    use serde_json::error::Category;
    match e.classify() {
        Category::Io => "io",
        Category::Syntax => "syntax",
        Category::Data => "data",
        Category::Eof => "eof",
    }
}

/// Something between an app and a component: ports plus a body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Widget {
    #[serde(rename = "in", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inputs: BTreeMap<PortName, Port>,
    #[serde(rename = "out", default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<PortName, Port>,
    pub body: Body,
}

impl Widget {
    pub fn ports(&self, dir: Dir) -> &BTreeMap<PortName, Port> {
        match dir {
            Dir::In => &self.inputs,
            Dir::Out => &self.outputs,
        }
    }
}

/// What a widget is made of.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Body {
    /// An axiom, implemented in Rust. `config` is opaque in phase 0.
    Primitive {
        kind: Kind,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        config: Value,
    },
    /// Data only: children and the wires between them.
    Compose {
        #[serde(default)]
        children: BTreeMap<ChildName, Widget>,
        #[serde(default)]
        wires: Vec<Wire>,
    },
}

/// The primitive kinds.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Kind {
    View,
    Expr,
    State,
    Egress,
    Secrets,
    Consent,
    Agent,
    Code,
    Process,
    /// Every `system.*` provider; which one is named in `config`.
    System,
}

impl Kind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::View => "view",
            Kind::Expr => "expr",
            Kind::State => "state",
            Kind::Egress => "egress",
            Kind::Secrets => "secrets",
            Kind::Consent => "consent",
            Kind::Agent => "agent",
            Kind::Code => "code",
            Kind::Process => "process",
            Kind::System => "system",
        }
    }
}

/// A port: its kind plus label declarations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "PortRepr", into = "PortRepr")]
#[schemars(with = "PortRepr")]
pub struct Port {
    pub kind: PortKind,
    /// Inputs only: may stay unwired.
    pub optional: bool,
    /// Primitives only: labels this port adds.
    pub adds: BTreeSet<Label>,
    /// Primitives only: the input-side ports this port depends on. `None` = all.
    pub depends_on: Option<BTreeSet<PortName>>,
    /// Primitives only: `Some` makes this port a sink accepting these labels.
    pub accepts: Option<BTreeSet<Label>>,
}

impl Port {
    pub fn new(kind: PortKind) -> Self {
        Self {
            kind,
            optional: false,
            adds: BTreeSet::new(),
            depends_on: None,
            accepts: None,
        }
    }

    pub fn tag(&self) -> PortKindTag {
        self.kind.tag()
    }
}

/// What travels through a port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortKind {
    /// A signal; every value carries origin labels.
    Data { schema: Schema },
    /// A capability that can be invoked.
    Call { input: Schema, output: Schema },
    /// An opaque A2UI surface: can be placed, not read.
    View,
}

impl PortKind {
    pub fn tag(&self) -> PortKindTag {
        match self {
            PortKind::Data { .. } => PortKindTag::Data,
            PortKind::Call { .. } => PortKindTag::Call,
            PortKind::View => PortKindTag::View,
        }
    }
}

/// A port kind without its schemas.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PortKindTag {
    Data,
    Call,
    View,
}

impl fmt::Display for PortKindTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            PortKindTag::Data => "data",
            PortKindTag::Call => "call",
            PortKindTag::View => "view",
        })
    }
}

/// The JSON form of a [`Port`].
#[derive(Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PortRepr {
    kind: PortKindTag,
    /// `data` only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    schema: Option<Schema>,
    /// `call` only: the argument type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<Schema>,
    /// `call` only: the result type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    output: Option<Schema>,
    #[serde(default, skip_serializing_if = "is_false")]
    optional: bool,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    adds: BTreeSet<Label>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    depends_on: Option<BTreeSet<PortName>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    accepts: Option<BTreeSet<Label>>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// The schemas of a port do not fit its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortShapeError;

impl fmt::Display for PortShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("data needs `schema`, call needs `input` and `output`, view needs none of them")
    }
}

impl TryFrom<PortRepr> for Port {
    type Error = PortShapeError;
    fn try_from(r: PortRepr) -> Result<Self, PortShapeError> {
        let kind = match (r.kind, r.schema, r.input, r.output) {
            (PortKindTag::Data, Some(schema), None, None) => PortKind::Data { schema },
            (PortKindTag::Call, None, Some(input), Some(output)) => {
                PortKind::Call { input, output }
            }
            (PortKindTag::View, None, None, None) => PortKind::View,
            _ => return Err(PortShapeError),
        };
        Ok(Port {
            kind,
            optional: r.optional,
            adds: r.adds,
            depends_on: r.depends_on,
            accepts: r.accepts,
        })
    }
}

impl From<Port> for PortRepr {
    fn from(p: Port) -> Self {
        let tag = p.kind.tag();
        let (schema, input, output) = match p.kind {
            PortKind::Data { schema } => (Some(schema), None, None),
            PortKind::Call { input, output } => (None, Some(input), Some(output)),
            PortKind::View => (None, None, None),
        };
        PortRepr {
            kind: tag,
            schema,
            input,
            output,
            optional: p.optional,
            adds: p.adds,
            depends_on: p.depends_on,
            accepts: p.accepts,
        }
    }
}

/// A type name. Two schemas match only if the strings are equal.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Schema(pub String);

impl fmt::Debug for Schema {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// An origin label, per provider: `[a-z][a-z0-9_-]*`, matched exactly.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String", extend("pattern" = r"^[a-z][a-z0-9_-]*$"))]
pub struct Label(String);

/// The string is not a valid [`Label`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelError;

impl fmt::Display for LabelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a label is [a-z][a-z0-9_-]*")
    }
}

impl Label {
    pub fn new(s: impl Into<String>) -> Result<Self, LabelError> {
        let s = s.into();
        let mut b = s.bytes();
        if matches!(b.next(), Some(b'a'..=b'z'))
            && b.all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
        {
            Ok(Self(s))
        } else {
            Err(LabelError)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Label {
    type Error = LabelError;
    fn try_from(s: String) -> Result<Self, LabelError> {
        Self::new(s)
    }
}

impl From<Label> for String {
    fn from(l: Label) -> String {
        l.0
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// A wire inside a compose scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Wire {
    pub from: Endpoint,
    pub to: Endpoint,
    /// Labels the scope owner lets through this wire. They are removed from the flow here.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub allow: BTreeSet<Label>,
}

/// One end of a wire: `self.in.<port>`, `<child>.out.<port>` and so on.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String", extend("pattern" = r"^[a-z][a-z0-9_]{0,63}\.(in|out)\.[a-z][a-z0-9_]{0,63}$"))]
pub struct Endpoint {
    pub node: Node,
    pub dir: Dir,
    pub port: PortName,
}

impl Endpoint {
    /// The port this endpoint names, seen from the scope at `scope`.
    pub fn at(&self, scope: &[ChildName]) -> PortPath {
        let mut widget = scope.to_vec();
        if let Node::Child(c) = &self.node {
            widget.push(c.clone());
        }
        PortPath {
            widget,
            dir: self.dir,
            port: self.port.clone(),
        }
    }
}

/// The string is not `<node>.<in|out>.<port>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointError;

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("an endpoint is <self|child>.<in|out>.<port>")
    }
}

impl TryFrom<String> for Endpoint {
    type Error = EndpointError;
    fn try_from(s: String) -> Result<Self, EndpointError> {
        let mut parts = s.split('.');
        let (Some(node), Some(dir), Some(port), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(EndpointError);
        };
        let node = match node {
            "self" => Node::This,
            n => Node::Child(ChildName::new(n).map_err(|_| EndpointError)?),
        };
        let dir = match dir {
            "in" => Dir::In,
            "out" => Dir::Out,
            _ => return Err(EndpointError),
        };
        let port = PortName::new(port).map_err(|_| EndpointError)?;
        Ok(Endpoint { node, dir, port })
    }
}

impl From<Endpoint> for String {
    fn from(e: Endpoint) -> String {
        format!("{e}")
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.node, self.dir, self.port)
    }
}

impl fmt::Debug for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

/// The widget an endpoint names: the scope itself or one of its direct children.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Node {
    This,
    Child(ChildName),
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Node::This => f.write_str("self"),
            Node::Child(c) => write!(f, "{c}"),
        }
    }
}

/// The side of a widget a port is on.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Dir {
    In,
    Out,
}

impl fmt::Display for Dir {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Dir::In => "in",
            Dir::Out => "out",
        })
    }
}

/// A port anywhere in the tree: the widget's path of child names from the root.
#[derive(
    Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct PortPath {
    pub widget: Vec<ChildName>,
    pub dir: Dir,
    pub port: PortName,
}

impl PortPath {
    pub fn new(widget: &[&str], dir: Dir, port: &str) -> Result<Self, base::NameError> {
        Ok(PortPath {
            widget: widget
                .iter()
                .map(|c| ChildName::new(*c))
                .collect::<Result<_, _>>()?,
            dir,
            port: PortName::new(port)?,
        })
    }
}

impl fmt::Display for PortPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.widget.is_empty() {
            f.write_str("self")?;
        }
        for (i, c) in self.widget.iter().enumerate() {
            if i > 0 {
                f.write_str("/")?;
            }
            write!(f, "{c}")?;
        }
        write!(f, ".{}.{}", self.dir, self.port)
    }
}

impl From<&PortPath> for Value {
    fn from(p: &PortPath) -> Value {
        serde_json::json!({
            "widget": p.widget.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "dir": p.dir,
            "port": p.port.as_str(),
        })
    }
}

/// JSON pointer to the widget at `path` inside a manifest.
pub(crate) fn widget_pointer(path: &[ChildName]) -> Pointer {
    let mut p = Pointer::root().push("widget");
    for c in path {
        p = p.push("body").push("compose").push("children").push(c);
    }
    p
}

pub(crate) fn port_pointer(path: &PortPath) -> Pointer {
    widget_pointer(&path.widget).push(path.dir).push(&path.port)
}

pub(crate) fn wire_pointer(scope: &[ChildName], index: usize) -> Pointer {
    widget_pointer(scope)
        .push("body")
        .push("compose")
        .push("wires")
        .push(index)
}

/// Every widget in the tree with its path, parents before children, in name order.
pub(crate) fn walk(root: &Widget) -> Vec<(Vec<ChildName>, &Widget)> {
    let mut out = vec![(Vec::new(), root)];
    let mut i = 0;
    while i < out.len() {
        let (path, w) = (out[i].0.clone(), out[i].1);
        if let Body::Compose { children, .. } = &w.body {
            for (name, child) in children {
                let mut p = path.clone();
                p.push(name.clone());
                out.push((p, child));
            }
        }
        i += 1;
    }
    out
}
