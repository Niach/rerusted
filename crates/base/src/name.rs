//! Names inside a widget: port names and child names.
//! Both match `[a-z][a-z0-9_]{0,63}`; a child cannot be called `self`.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Why a string is not a valid name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NameError {
    /// Not of the form `[a-z][a-z0-9_]{0,63}`.
    Syntax,
    /// `self` is reserved for the scope itself.
    Reserved,
}

impl fmt::Display for NameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NameError::Syntax => {
                "a name is 1 to 64 characters of [a-z0-9_] and starts with a letter"
            }
            NameError::Reserved => "`self` is reserved and cannot name a child",
        })
    }
}

fn valid(s: &str) -> bool {
    let mut bytes = s.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && s.len() <= 64
        && bytes.all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

macro_rules! names {
    ($($(#[$doc:meta])* $name:ident, $reserved:expr;)*) => {$(
        $(#[$doc])*
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema)]
        #[serde(try_from = "String", into = "String")]
        #[schemars(with = "String", extend("pattern" = r"^[a-z][a-z0-9_]{0,63}$"))]
        pub struct $name(String);

        impl $name {
            pub fn new(s: impl Into<String>) -> Result<Self, NameError> {
                let s = s.into();
                if !valid(&s) {
                    return Err(NameError::Syntax);
                }
                if $reserved.contains(&s.as_str()) {
                    return Err(NameError::Reserved);
                }
                Ok(Self(s))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = NameError;
            fn try_from(s: String) -> Result<Self, NameError> {
                Self::new(s)
            }
        }

        impl From<$name> for String {
            fn from(n: $name) -> String {
                n.0
            }
        }

        impl From<$name> for serde_json::Value {
            fn from(n: $name) -> Self {
                serde_json::Value::String(n.0)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{:?}", self.0)
            }
        }
    )*};
}

const NONE: [&str; 0] = [];

names! {
    /// The name of a port on a widget.
    PortName, NONE;
    /// The name of a child inside a compose widget.
    ChildName, ["self"];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(PortName::new("posts").is_ok());
        assert!(PortName::new("a_1").is_ok());
        assert!(PortName::new("self").is_ok());
        assert_eq!(PortName::new(""), Err(NameError::Syntax));
        assert_eq!(PortName::new("1a"), Err(NameError::Syntax));
        assert_eq!(PortName::new("a.b"), Err(NameError::Syntax));
        assert_eq!(PortName::new("A"), Err(NameError::Syntax));
        assert_eq!(PortName::new("a".repeat(65)), Err(NameError::Syntax));
        assert!(PortName::new("a".repeat(64)).is_ok());
        assert_eq!(ChildName::new("self"), Err(NameError::Reserved));
    }
}
