//! The done-when of the VM runner: two monitors, and a scripted unplug and
//! replug moves a window away and back.
//!
//! Needs KVM and builds the base image on first use, so it is ignored by
//! default (tier 3 runs per milestone, not per PR):
//!
//! ```text
//! cargo test -p vm --test hotplug -- --ignored
//! ```
//!
//! The session is the stock stand-in (sway); the window's output is read
//! from sway's tree. Once the workspace has its own compositor this test
//! asks that compositor instead.

use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::Value;
use vm::{Family, Rig, STAND_IN_SESSION};

const SWAY: &str = "SWAYSOCK=/run/user/$(id -u)/sway-ipc.$(id -u).$(pgrep -x sway).sock swaymsg";

/// The name of the output that holds the window with this app id.
fn output_of(tree: &Value, app_id: &str, output: Option<&str>) -> Option<String> {
    let output = match tree["type"].as_str() {
        Some("output") => tree["name"].as_str(),
        _ => output,
    };
    if tree["app_id"] == app_id {
        return output.map(str::to_string);
    }
    ["nodes", "floating_nodes"]
        .into_iter()
        .filter_map(|key| tree[key].as_array())
        .flatten()
        .find_map(|child| output_of(child, app_id, output))
}

fn window_output(rig: &Rig) -> Option<String> {
    let tree = rig.ssh(&format!("{SWAY} -t get_tree -r")).ok()?;
    output_of(&serde_json::from_str(&tree).ok()?, "foot", None)
}

fn wait_for_window_on(rig: &Rig, output: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let found = window_output(rig);
        if found.as_deref() == Some(output) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "window is on {found:?}, expected {output}"
        );
        sleep(Duration::from_millis(200));
    }
}

#[test]
#[ignore = "needs KVM and a base image; run with --ignored"]
fn unplug_moves_the_window_away_and_replug_brings_it_back() {
    let rig = Rig::new(Family::Apt).unwrap();
    rig.up(&vm::target_dir().unwrap()).unwrap();
    rig.session(&format!("echo session-says-hello; exec {STAND_IN_SESSION}"))
        .unwrap();

    // Both monitors are real DRM connectors in the guest.
    let connectors = rig
        .ssh("cat /sys/class/drm/card*-Virtual-*/status")
        .unwrap();
    assert_eq!(connectors, "connected\nconnected\n");

    // The host's target/ is there, read-only, and nothing can compile.
    rig.ssh("test -d /mnt/target/debug && ! touch /mnt/target/probe 2>/dev/null")
        .unwrap();
    rig.ssh("! command -v cc gcc rustc cargo").unwrap();

    // The session is a logind session on seat0.
    let deadline = Instant::now() + Duration::from_secs(20);
    while rig.ssh(&format!("{SWAY} -t get_version")).is_err() {
        assert!(
            Instant::now() < deadline,
            "the session did not come up:\n{}",
            rig.session_log().unwrap_or_default()
        );
        sleep(Duration::from_millis(200));
    }
    let sessions = rig.ssh("loginctl list-sessions --no-legend").unwrap();
    assert!(
        sessions
            .lines()
            .any(|line| line.contains("seat0") && line.contains("tty7")),
        "{sessions}"
    );

    // What the session prints comes back as its log.
    assert!(rig.session_log().unwrap().contains("session-says-hello"));

    // A window on the second monitor.
    rig.ssh(&format!("{SWAY} -q workspace 2 && {SWAY} -q exec foot"))
        .unwrap();
    wait_for_window_on(&rig, "Virtual-2");

    rig.unplug(1).unwrap();
    wait_for_window_on(&rig, "Virtual-1");
    assert_eq!(
        rig.ssh("cat /sys/class/drm/card*-Virtual-2/status")
            .unwrap(),
        "disconnected\n"
    );

    rig.plug(1, 1280, 800).unwrap();
    wait_for_window_on(&rig, "Virtual-2");
}
