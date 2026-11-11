//! Primitive manifests: what each primitive kind may declare. Descriptions only.

use crate::model::{Kind, PortKindTag};

use PortKindTag::{Call, Data, View};

/// What a primitive kind may declare and how the checker treats it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PrimitiveSpec {
    pub kind: Kind,
    /// Port kinds an instance may declare as inputs.
    pub inputs: &'static [PortKindTag],
    /// Port kinds an instance may declare as outputs.
    pub outputs: &'static [PortKindTag],
    /// C7: every output-side port depends on every input-side port; no `depends_on`.
    pub black_box: bool,
    /// C0: every input-side port declares `accepts`.
    pub sink: bool,
    /// C0: every output-side port declares a non-empty `adds`.
    pub must_label: bool,
    /// C0: only a direct child of the manifest root.
    pub root_only: bool,
    /// C5: a data cycle through this kind is allowed.
    pub breaks_cycles: bool,
    pub doc: &'static str,
}

const fn spec_of(
    kind: Kind,
    inputs: &'static [PortKindTag],
    outputs: &'static [PortKindTag],
    doc: &'static str,
) -> PrimitiveSpec {
    PrimitiveSpec {
        kind,
        inputs,
        outputs,
        black_box: false,
        sink: false,
        must_label: false,
        root_only: false,
        breaks_cycles: false,
        doc,
    }
}

const VIEW: PrimitiveSpec = spec_of(
    Kind::View,
    &[Data, View, Call],
    &[View, Data],
    "Renders an A2UI tree. Data inputs are bindings, view inputs are slots, call inputs are actions; data outputs are events.",
);
const EXPR: PrimitiveSpec = spec_of(
    Kind::Expr,
    &[Data],
    &[Data],
    "A pure, terminating expression: map, filter, sort, join.",
);
const STATE: PrimitiveSpec = PrimitiveSpec {
    breaks_cycles: true,
    ..spec_of(
        Kind::State,
        &[Data, Call],
        &[Data, Call],
        "A cell or record set in a scope.",
    )
};
const EGRESS: PrimitiveSpec = PrimitiveSpec {
    sink: true,
    must_label: true,
    ..spec_of(
        Kind::Egress,
        &[Call, Data],
        &[Call, Data],
        "One HTTP or MCP endpoint with a fixed host. Uses a secret through a call input; never holds the token.",
    )
};
const SECRETS: PrimitiveSpec = PrimitiveSpec {
    root_only: true,
    ..spec_of(
        Kind::Secrets,
        &[],
        &[Call],
        "Holds tokens. Never has a data port; offers secret use as calls.",
    )
};
const CONSENT: PrimitiveSpec = PrimitiveSpec {
    root_only: true,
    ..spec_of(
        Kind::Consent,
        &[Call],
        &[Call],
        "The only place a permission prompt is drawn. Offers grants as calls.",
    )
};
const AGENT: PrimitiveSpec = PrimitiveSpec {
    black_box: true,
    ..spec_of(
        Kind::Agent,
        &[Call, Data],
        &[Data, View],
        "The agent loop. Its tools are exactly its call inputs.",
    )
};
const CODE: PrimitiveSpec = PrimitiveSpec {
    black_box: true,
    ..spec_of(
        Kind::Code,
        &[Data, Call, View],
        &[Data, Call, View],
        "A WASM component.",
    )
};
const PROCESS: PrimitiveSpec = PrimitiveSpec {
    black_box: true,
    ..spec_of(
        Kind::Process,
        &[Data, Call],
        &[Call, View],
        "A sandboxed native process. Its commands are call outputs, its window is a view output.",
    )
};
const SYSTEM: PrimitiveSpec = PrimitiveSpec {
    sink: true,
    must_label: true,
    ..spec_of(
        Kind::System,
        &[Call, Data],
        &[Call, Data],
        "A hardware or system service; the provider is named in config.",
    )
};

/// The registry entry for a primitive kind.
pub fn spec(kind: Kind) -> &'static PrimitiveSpec {
    match kind {
        Kind::View => &VIEW,
        Kind::Expr => &EXPR,
        Kind::State => &STATE,
        Kind::Egress => &EGRESS,
        Kind::Secrets => &SECRETS,
        Kind::Consent => &CONSENT,
        Kind::Agent => &AGENT,
        Kind::Code => &CODE,
        Kind::Process => &PROCESS,
        Kind::System => &SYSTEM,
    }
}
