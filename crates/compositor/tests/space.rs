//! The headless backend, driven the way an agent drives its space.
#![cfg(target_os = "linux")]

mod common;

use std::{
    io::Read,
    process::Stdio,
    time::{Duration, Instant},
};

use common::{
    Rig, drain,
    probe::{BACKGROUNDS, Event, Probe, ProbeConfig, SQUARE, square},
    wait_frame, wait_until, wait_window,
};
use compositor::{DeskKind, OutputKind, Rect, SpaceConfig, keys};

fn no_x() -> SpaceConfig {
    SpaceConfig {
        xwayland: false,
        ..SpaceConfig::default()
    }
}

/// Kills a child process when the test ends, passed or failed.
struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn two_outputs_and_two_seats_exist_and_each_client_sees_only_its_desk() {
    let rig = Rig::start();
    let space = rig.handle.create_space(no_x()).unwrap();

    let outputs = rig.handle.outputs().unwrap();
    let kinds: Vec<_> = outputs
        .iter()
        .map(|o| (o.name.as_str(), o.desk, o.kind))
        .collect();
    assert_eq!(
        kinds,
        [
            ("out-1", rig.handle.person().id(), OutputKind::Headless),
            ("out-2", rig.handle.person().id(), OutputKind::Headless),
            ("space-1", space.id(), OutputKind::Virtual),
        ]
    );
    assert_eq!(
        outputs[1].geometry,
        Rect {
            x: 960,
            y: 0,
            w: 960,
            h: 600
        }
    );
    let seats: Vec<_> = rig
        .handle
        .seats()
        .unwrap()
        .into_iter()
        .map(|s| (s.name, s.desk))
        .collect();
    assert_eq!(
        seats,
        [
            ("seat-person".to_string(), rig.handle.person().id()),
            ("seat-space-1".to_string(), space.id())
        ]
    );
    assert_eq!(space.kind(), DeskKind::Space);

    let person = Probe::spawn(ProbeConfig::new(rig.handle.person().socket(), "person app"));
    let agent = Probe::spawn(ProbeConfig::new(space.socket(), "agent app"));
    wait_until("the clients to learn their seat and outputs", || {
        person.globals().output_names.len() == 2 && agent.globals().output_names.len() == 1
    });

    let seen = person.globals();
    assert_eq!((seen.seats, seen.outputs, seen.layer_shell), (1, 2, true));
    assert_eq!(seen.seat_names, ["seat-person"]);
    assert_eq!(seen.output_names, ["out-1", "out-2"]);

    let seen = agent.globals();
    assert_eq!((seen.seats, seen.outputs, seen.layer_shell), (1, 1, false));
    assert_eq!(seen.seat_names, ["seat-space-1"]);
    assert_eq!(seen.output_names, ["space-1"]);
}

#[test]
fn a_wayland_app_in_a_space_is_clicked_typed_into_and_read_back() {
    let rig = Rig::start();
    let space = rig.handle.create_space(no_x()).unwrap();
    let frames = space.frames(0).unwrap();

    let probe = Probe::spawn(ProbeConfig::new(space.socket(), "agent app"));
    wait_frame(&frames, "the window appears", |f| {
        f.pixel(400, 300) == BACKGROUNDS[0]
    });

    let windows = space.windows().unwrap();
    assert_eq!(windows.len(), 1);
    assert_eq!(
        (windows[0].title.as_str(), windows[0].app_id.as_str()),
        ("agent app", "probe")
    );
    assert_eq!(
        windows[0].geometry,
        Rect {
            x: 0,
            y: 0,
            w: 800,
            h: 600
        },
        "a window fills its space"
    );

    // Click: the app gets the press where it was aimed, and the change comes back as a frame.
    space.click(300.0, 200.0).unwrap();
    wait_frame(&frames, "the click changes the window", |f| {
        f.pixel(300, 200) == BACKGROUNDS[1]
    });
    assert!(probe.events().contains(&Event::Button {
        x: 300.0,
        y: 200.0,
        button: keys::BTN_LEFT,
        pressed: true
    }));

    // Type: the app receives the text, and the last character's mark shows up in a frame.
    let text = "Hello, agent #1!\n";
    space.type_text(text).unwrap();
    probe.wait_for("the typed text", |_| probe.typed() == text);
    let last = square(text.chars().count() - 1);
    let (x, y) = ((last.0 + 8) as u32, (last.1 + 8) as u32);
    wait_frame(&frames, "the typed text shows", |f| f.pixel(x, y) == SQUARE);
}

#[test]
fn an_x11_app_runs_on_the_spaces_own_xwayland() {
    let rig = Rig::start();
    let space = rig.handle.create_space(SpaceConfig::default()).unwrap();
    assert!(space.x11_display().is_some());
    let frames = space.frames(0).unwrap();
    let out = rig.dir.path().join("xterm-out");

    let child = space
        .command("xterm")
        .env_remove("WAYLAND_DISPLAY")
        .stderr(Stdio::null())
        .spawn();
    let _child = Child(child.expect("xterm is installed"));
    wait_until("the xterm window", || {
        space.windows().unwrap().iter().any(|w| w.x11)
    });
    wait_frame(&frames, "xterm draws", |_| true);
    drain(&frames, Duration::from_millis(500));

    space.click(200.0, 150.0).unwrap();
    space
        .type_text(&format!("echo driven > {}\n", out.display()))
        .unwrap();
    wait_until("the shell in xterm to run the typed command", || {
        std::fs::read_to_string(&out).is_ok_and(|s| s == "driven\n")
    });
    wait_frame(&frames, "typing changes the terminal", |f| {
        !f.damage.is_empty()
    });
}

#[test]
fn a_gtk_app_in_a_space_returns_what_was_typed() {
    let rig = Rig::start();
    let space = rig.handle.create_space(no_x()).unwrap();
    let frames = space.frames(0).unwrap();

    let child = space
        .command("zenity")
        .args(["--entry", "--text", "Name"])
        .env("GDK_BACKEND", "wayland")
        .env("NO_AT_BRIDGE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = Child(child.expect("zenity is installed"));
    wait_until("the dialog", || !space.windows().unwrap().is_empty());
    wait_frame(&frames, "the dialog draws", |_| true);
    drain(&frames, Duration::from_millis(500));

    let geo = space.windows().unwrap()[0].geometry;
    space
        .click((geo.x + geo.w / 2) as f64, (geo.y + geo.h / 2) as f64)
        .unwrap();
    space.type_text("from the agent").unwrap();
    wait_frame(&frames, "the entry shows the text", |f| {
        !f.damage.is_empty()
    });
    space.type_text("\n").unwrap();

    let mut answer = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut answer)
        .unwrap();
    assert_eq!(answer.trim(), "from the agent");
}

#[test]
fn the_persons_seat_is_untouched_while_a_space_is_driven() {
    let rig = Rig::start();
    let person_desk = rig.handle.person();

    // The person works in a window: pointer over it, keyboard in it, something copied.
    let mut config = ProbeConfig::new(person_desk.socket(), "person app");
    config.selection_on_focus = Some("the person's clipboard".into());
    let person = Probe::spawn(config);
    wait_window(person_desk);
    person.wait_for("the person's copy", |e| {
        e.iter().any(|e| matches!(e, Event::Selection(Some(_))))
    });
    person_desk.pointer_move(200.0, 200.0).unwrap();
    person.wait_for("the pointer", |e| e.contains(&Event::PointerEnter));
    wait_until("the selection to be recorded", || {
        person_desk.seat_snapshot().unwrap().selection.is_some()
    });

    let before = person_desk.seat_snapshot().unwrap();
    let events_before = person.events();
    assert!(before.keyboard_focus.is_some() && before.pointer_focus.is_some());

    // An agent works in its space: moves, clicks, types, copies.
    let space = rig.handle.create_space(no_x()).unwrap();
    let mut config = ProbeConfig::new(space.socket(), "agent app");
    config.selection_on_click = Some("the agent's clipboard".into());
    let agent = Probe::spawn(config);
    wait_window(&space);
    for step in 0..50 {
        space
            .pointer_move(100.0 + step as f64 * 5.0, 100.0 + step as f64 * 3.0)
            .unwrap();
    }
    space.click(250.0, 250.0).unwrap();
    space.type_text("typed by the agent\n").unwrap();
    agent.wait_for("the agent's input", |_| {
        agent.typed() == "typed by the agent\n"
    });
    agent.wait_for("the agent's copy", |e| {
        e.iter().any(|e| matches!(e, Event::Selection(Some(_))))
    });
    assert_eq!(agent.paste().as_deref(), Some("the agent's clipboard"));

    // Nothing reached the person's client, and nothing about the person's seat moved.
    assert_eq!(
        person.events(),
        events_before,
        "the person's client received events"
    );
    assert_eq!(
        person_desk.seat_snapshot().unwrap(),
        before,
        "the person's seat changed"
    );
    assert_eq!(person.paste().as_deref(), Some("the person's clipboard"));

    // The agent's seat is the one that did the work.
    let agent_seat = space.seat_snapshot().unwrap();
    assert!(agent_seat.input_events > 50);
    assert_eq!(agent_seat.pointer, (250.0, 250.0));
    assert_ne!(agent_seat.keyboard_focus, None);
}

#[test]
fn frames_are_delivered_only_on_damage() {
    let rig = Rig::start();
    let space = rig.handle.create_space(no_x()).unwrap();
    let frames = space.frames(0).unwrap();
    let probe = Probe::spawn(ProbeConfig::new(space.socket(), "agent app"));
    wait_frame(&frames, "the window appears", |f| {
        f.pixel(400, 300) == BACKGROUNDS[0]
    });
    drain(&frames, Duration::from_millis(300));

    // Idle: no frame and no render at all.
    let renders = rig.handle.stats().unwrap().renders;
    assert!(
        frames.recv_timeout(Duration::from_secs(1)).is_err(),
        "an idle space delivered a frame"
    );
    assert_eq!(
        rig.handle.stats().unwrap().renders,
        renders,
        "an idle space was rendered"
    );

    // Moving the pointer changes nothing on screen, so it delivers nothing.
    space.pointer_move(50.0, 300.0).unwrap();
    space.pointer_move(60.0, 310.0).unwrap();
    probe.wait_for("the motion", |e| {
        e.iter().any(|e| matches!(e, Event::Motion(..)))
    });
    assert!(
        frames.recv_timeout(Duration::from_millis(300)).is_err(),
        "pointer motion delivered a frame"
    );

    // One change: one frame, and its damage is exactly what changed.
    let sent = Instant::now();
    space.type_text("x").unwrap();
    let frame = wait_frame(&frames, "the change", |_| true);
    let (x, y, w, h) = square(0);
    assert_eq!(frame.damage_bounds(), Rect { x, y, w, h });
    assert_eq!(frame.pixel(x as u32 + 8, y as u32 + 8), SQUARE);
    assert!(frame.time.duration_since(sent) < Duration::from_secs(1));
    assert!(
        frames.recv_timeout(Duration::from_millis(300)).is_err(),
        "one change delivered two frames"
    );
}

#[test]
fn closing_a_space_removes_its_seat_output_and_socket() {
    let rig = Rig::start();
    let space = rig.handle.create_space(SpaceConfig::default()).unwrap();
    let socket = space.socket().to_path_buf();
    assert!(socket.exists());
    space.close().unwrap();
    assert!(!socket.exists());
    assert_eq!(rig.handle.outputs().unwrap().len(), 2);
    assert_eq!(rig.handle.seats().unwrap().len(), 1);
    assert!(space.seat_snapshot().is_err());
}

/// The sandboxed form of a space is later work (#RUST-20). This shows the two
/// parts of it that need nothing from the compositor: an app in a space runs
/// with a runtime directory and a D-Bus session bus of its own, because it
/// reaches its desk through the socket's absolute path.
#[test]
fn a_space_app_runs_with_its_own_runtime_dir_and_session_bus() {
    let rig = Rig::start();
    let space = rig.handle.create_space(no_x()).unwrap();
    let frames = space.frames(0).unwrap();
    let runtime = rig.dir.path().join("space-runtime");
    std::fs::create_dir(&runtime).unwrap();
    let bus_file = rig.dir.path().join("bus");

    let script = format!(
        "echo \"$DBUS_SESSION_BUS_ADDRESS\" > {}; exec zenity --entry --text Name",
        bus_file.display()
    );
    let child = space
        .command("dbus-run-session")
        .args(["--", "sh", "-c", &script])
        .env("XDG_RUNTIME_DIR", &runtime)
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .env("GDK_BACKEND", "wayland")
        .env("NO_AT_BRIDGE", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = Child(child.expect("dbus-run-session is installed"));
    wait_window(&space);
    wait_frame(&frames, "the dialog draws", |_| true);
    drain(&frames, Duration::from_millis(500));

    space.type_text("sub-session\n").unwrap();
    let mut answer = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut answer)
        .unwrap();
    assert_eq!(answer.trim(), "sub-session");

    let bus = std::fs::read_to_string(&bus_file).unwrap();
    assert!(
        bus.trim().starts_with("unix:"),
        "the app has a session bus: {bus:?}"
    );
    assert_ne!(
        Some(bus.trim().to_string()),
        std::env::var("DBUS_SESSION_BUS_ADDRESS").ok(),
        "and it is not the person's"
    );
}
