use base::{
    ChildName, Class, Code, DeviceId, Fault, FaultId, Pointer, PortName, Remedy, Secret, SessionId,
    Span, TeamId, UserId, WidgetId,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::json;

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) {
    let text = serde_json::to_string(value).unwrap();
    let back: T = serde_json::from_str(&text).unwrap();
    assert_eq!(&back, value);
}

#[test]
fn ids_roundtrip() {
    roundtrip(&WidgetId::new());
    roundtrip(&UserId::new());
    roundtrip(&DeviceId::new());
    roundtrip(&TeamId::new());
    roundtrip(&SessionId::new());
    roundtrip(&FaultId::new());
    roundtrip(&PortName::new("posts").unwrap());
    roundtrip(&ChildName::new("feed").unwrap());

    let id = WidgetId::new();
    assert_eq!(id.as_uuid().get_version_num(), 7);
    let text = id.to_string();
    assert_eq!(text.len(), 36);
    assert_eq!(text, text.to_lowercase());
    assert_eq!(serde_json::to_value(id).unwrap(), json!(text));
    assert_eq!(text.parse::<WidgetId>().unwrap(), id);
}

#[test]
fn invalid_names_are_rejected_on_deserialize() {
    assert!(serde_json::from_str::<ChildName>("\"self\"").is_err());
    assert!(serde_json::from_str::<PortName>("\"Posts\"").is_err());
    assert!(serde_json::from_str::<Code>("\"graph\"").is_err());
    assert!(serde_json::from_str::<WidgetId>("\"nope\"").is_err());
}

fn leaf(code: &'static str, class: Class) -> Fault {
    Fault::new(Code::of(code), class, "A leaf.").field("n", 1)
}

#[test]
fn nested_fault_roundtrips_identical() {
    let fault = Fault::new(Code::of("graph.wire.type_mismatch"), Class::Fixable, "The schemas differ.")
        .field("from", "list<twitter.post>")
        .field("to", "list<discord.message>")
        .field("count", 2)
        .field("ratio", 0.5)
        .field("ok", false)
        .field("labels", json!(["twitter", "discord"]))
        .field("sink", json!({ "widget": ["feed"], "dir": "in", "port": "rows" }))
        .pointer(Pointer::root().push("widget").push("body"))
        .wire(Pointer::root().push("wires").push(0))
        .wire(Pointer::root().push("wires").push(3))
        .cause(
            leaf("net.http.timeout", Class::Retryable)
                .cause(leaf("net.dns.failed", Class::Retryable).cause(leaf("base.bug.unreachable", Class::Bug))),
        )
        .span(Span::Session { id: SessionId::new() })
        .span(Span::Widget { id: WidgetId::new(), path: vec![ChildName::new("feed").unwrap()] })
        .span(Span::ToolCall { tool: "tasks.list".into(), call: "call_1".into() })
        .remedy(
            Remedy::new(Code::of("graph.wire.allow"))
                .param("wire", Pointer::root().push("wires").push(3))
                .param("labels", json!(["twitter"])),
        );

    // three causes, nested
    assert_eq!(fault.causes[0].causes[0].causes.len(), 1);
    roundtrip(&fault);

    let value = serde_json::to_value(&fault).unwrap();
    assert_eq!(value["class"], "fixable");
    assert_eq!(value["code"], "graph.wire.type_mismatch");
    assert_eq!(value["trace"][2]["span"], "tool_call");
    assert_eq!(fault.to_string(), "graph.wire.type_mismatch: The schemas differ.");
}

#[test]
fn secret_never_shows() {
    #[derive(Serialize, Debug)]
    struct Holder {
        name: &'static str,
        token: Secret<String>,
    }
    let h = Holder { name: "twitter", token: Secret::new("hunter2-token".into()) };
    let debug = format!("{h:?} {:?}", h.token);
    let display = format!("{}", h.token);
    let json = serde_json::to_string(&h).unwrap();
    for out in [&debug, &display, &json] {
        assert!(!out.contains("hunter2"), "{out}");
    }
    assert!(json.contains("<redacted>"));
    assert_eq!(h.token.expose(), "hunter2-token");
}
