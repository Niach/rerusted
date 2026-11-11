//! [`Fault`]: the one error type of the project.
//!
//! A fault gives an agent complete context and crosses FFI, p2p and MCP
//! unchanged: it serialises to JSON and back without loss.

use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ChildName, FaultId, SessionId, WidgetId};

/// A fault.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Fault {
    /// Unique, so the fault can be fetched later with its surrounding events.
    pub id: FaultId,
    /// Stable and dotted, for example `graph.wire.type_mismatch`.
    pub code: Code,
    pub class: Class,
    /// One sentence for a person.
    pub message: String,
    /// Typed context. Never pre-formatted strings.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fields: BTreeMap<String, Value>,
    #[serde(default, skip_serializing_if = "Path::is_empty")]
    pub path: Path,
    /// The full chain of causes, each a fault.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub causes: Vec<Fault>,
    /// Span trace, outermost first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<Span>,
    /// Machine-readable next steps.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remedies: Vec<Remedy>,
}

impl Fault {
    pub fn new(code: Code, class: Class, message: impl Into<String>) -> Self {
        Self {
            id: FaultId::new(),
            code,
            class,
            message: message.into(),
            fields: BTreeMap::new(),
            path: Path::default(),
            causes: Vec::new(),
            trace: Vec::new(),
            remedies: Vec::new(),
        }
    }

    pub fn field(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }

    pub fn pointer(mut self, pointer: Pointer) -> Self {
        self.path.pointer = Some(pointer);
        self
    }

    pub fn wire(mut self, wire: Pointer) -> Self {
        self.path.wires.push(wire);
        self
    }

    pub fn cause(mut self, cause: Fault) -> Self {
        self.causes.push(cause);
        self
    }

    pub fn span(mut self, span: Span) -> Self {
        self.trace.push(span);
        self
    }

    pub fn remedy(mut self, remedy: Remedy) -> Self {
        self.remedies.push(remedy);
        self
    }
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Fault {}

/// Who can act on a fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Class {
    /// Trying again may succeed.
    Retryable,
    /// The person can fix it.
    Fixable,
    /// A permission is missing.
    Permission,
    /// A bug in the program.
    Bug,
}

/// A stable dotted code: two or more segments of `[a-z][a-z0-9_]*`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String", regex(pattern = r"^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$"))]
pub struct Code(String);

/// The string is not a valid [`Code`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodeError;

impl fmt::Display for CodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a code is two or more dotted segments of [a-z][a-z0-9_]*")
    }
}

impl Code {
    pub fn new(s: impl Into<String>) -> Result<Self, CodeError> {
        let s = s.into();
        let segment = |p: &str| {
            let mut b = p.bytes();
            matches!(b.next(), Some(b'a'..=b'z'))
                && b.all(|c| matches!(c, b'a'..=b'z' | b'0'..=b'9' | b'_'))
        };
        if s.split('.').count() >= 2 && s.split('.').all(segment) {
            Ok(Self(s))
        } else {
            Err(CodeError)
        }
    }

    /// For codes written as literals in the source. Panics on an invalid literal.
    pub fn of(s: &'static str) -> Self {
        match Self::new(s) {
            Ok(c) => c,
            Err(_) => panic!("invalid fault code literal"),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Code {
    type Error = CodeError;
    fn try_from(s: String) -> Result<Self, CodeError> {
        Self::new(s)
    }
}

impl From<Code> for String {
    fn from(c: Code) -> String {
        c.0
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// Where a fault happened.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Path {
    /// The place in the document.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<Pointer>,
    /// For graphs: a pointer to each wire on the chain, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wires: Vec<Pointer>,
}

impl Path {
    pub fn is_empty(&self) -> bool {
        self.pointer.is_none() && self.wires.is_empty()
    }
}

/// An RFC 6901 JSON pointer: empty, or `/`-separated escaped segments.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String", into = "String")]
#[schemars(with = "String")]
pub struct Pointer(String);

/// The string is not an RFC 6901 pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointerError;

impl fmt::Display for PointerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a JSON pointer is empty or starts with /")
    }
}

impl Pointer {
    /// The whole document.
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Appends one segment, escaping `~` and `/`.
    pub fn push(mut self, segment: impl fmt::Display) -> Self {
        self.0.push('/');
        for c in segment.to_string().chars() {
            match c {
                '~' => self.0.push_str("~0"),
                '/' => self.0.push_str("~1"),
                c => self.0.push(c),
            }
        }
        self
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Pointer {
    type Error = PointerError;
    fn try_from(s: String) -> Result<Self, PointerError> {
        if s.is_empty() || s.starts_with('/') {
            Ok(Self(s))
        } else {
            Err(PointerError)
        }
    }
}

impl From<Pointer> for String {
    fn from(p: Pointer) -> String {
        p.0
    }
}

impl From<Pointer> for Value {
    fn from(p: Pointer) -> Value {
        Value::String(p.0)
    }
}

impl fmt::Display for Pointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for Pointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.0)
    }
}

/// One span of the trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "span", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Span {
    Session { id: SessionId },
    Widget { id: WidgetId, path: Vec<ChildName> },
    ToolCall { tool: String, call: String },
}

/// A machine-readable next step, for example `graph.wire.allow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Remedy {
    pub action: Code,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, Value>,
}

impl Remedy {
    pub fn new(action: Code) -> Self {
        Self { action, params: BTreeMap::new() }
    }

    pub fn param(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes() {
        assert!(Code::new("graph.wire.type_mismatch").is_ok());
        assert!(Code::new("a.b").is_ok());
        assert!(Code::new("graph").is_err());
        assert!(Code::new("graph.").is_err());
        assert!(Code::new("Graph.x").is_err());
        assert!(Code::new("graph..x").is_err());
        assert!(Code::new("graph.1x").is_err());
    }

    #[test]
    fn pointers() {
        let p = Pointer::root().push("a/b").push("c~d").push(3);
        assert_eq!(p.as_str(), "/a~1b/c~0d/3");
        assert!(Pointer::try_from("x".to_string()).is_err());
    }
}
