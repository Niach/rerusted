//! gpui as a client: one window, one panel and one overlay (#RUST-23).
//!
//! Needs the binary from `spikes/gpui-shell`, which lives outside the
//! workspace because it builds zed's gpui from git:
//!
//! ```text
//! cargo build --manifest-path spikes/gpui-shell/Cargo.toml
//! GPUI_SHELL=spikes/gpui-shell/target/debug/gpui-shell \
//!     cargo test -p compositor --test gpui -- --ignored
//! ```
//!
//! `GPUI_SHOTS=<dir>` also writes screenshots of the output.
#![cfg(target_os = "linux")]

mod common;

use std::{path::PathBuf, process::Stdio};

use common::{Rig, wait_frame, wait_until};
use compositor::{Frame, Rect};

const BACKGROUND: (u8, u8, u8) = (0x0f, 0x08, 0x07);
const CARD: (u8, u8, u8) = (0x1e, 0x15, 0x12);
const ACCENT: (u8, u8, u8) = (0xba, 0x4f, 0x1f);

fn near(a: (u8, u8, u8), b: (u8, u8, u8)) -> bool {
    let d = |x: u8, y: u8| x.abs_diff(y) <= 4;
    d(a.0, b.0) && d(a.1, b.1) && d(a.2, b.2)
}

fn shot(frame: &Frame, name: &str) {
    if let Some(dir) = std::env::var_os("GPUI_SHOTS") {
        frame.write_png(&PathBuf::from(dir).join(name)).unwrap();
    }
}

struct Child(std::process::Child);

impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "needs the gpui-shell binary, see the top of this file"]
fn a_gpui_window_a_gpui_panel_and_a_gpui_overlay_are_ordinary_clients() {
    let binary = std::env::var_os("GPUI_SHELL").expect("GPUI_SHELL names the gpui-shell binary");
    let binary = std::fs::canonicalize(binary).expect("GPUI_SHELL exists");
    let rig = Rig::start();
    let person = rig.handle.person();
    let frames = person.frames(0).unwrap();

    let child = person
        .command(binary.to_str().unwrap())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let _child = Child(child.expect("gpui-shell starts"));

    // The panel and the overlay arrive through layer shell, on the layers they asked for.
    let layers = || {
        let mut found = rig.handle.outputs().unwrap()[0].layers.clone();
        found.sort();
        found
    };
    let wanted = [
        ("overlay".to_string(), "Overlay".to_string()),
        ("panel".to_string(), "Top".to_string()),
    ];
    wait_until("gpui's layer surfaces", || layers() == wanted);

    // The panel's exclusive zone takes 32 px off the top of the output.
    wait_until("the panel's exclusive zone", || {
        rig.handle.outputs().unwrap()[0].usable
            == Rect {
                x: 0,
                y: 32,
                w: 960,
                h: 568,
            }
    });
    assert_eq!(
        rig.handle.outputs().unwrap()[1].usable,
        Rect {
            x: 0,
            y: 0,
            w: 960,
            h: 600
        }
    );

    // The window is an xdg toplevel, placed below the panel.
    let window = || {
        person
            .windows()
            .unwrap()
            .into_iter()
            .find(|w| w.app_id == "gpui-window")
    };
    wait_until("the gpui window", || {
        window().is_some_and(|w| w.geometry.w > 0)
    });
    let geo = window().unwrap().geometry;
    assert!(geo.y >= 32, "the window is below the panel: {geo:?}");

    // All three draw: panel at the top edge, overlay at the bottom right, window in between.
    let (cx, cy) = ((geo.x + geo.w / 2) as u32, (geo.y + geo.h * 3 / 4) as u32);
    let drawn = |f: &Frame| {
        near(f.pixel(600, 10), BACKGROUND)
            && near(f.pixel(960 - 24 - 20, 600 - 24 - 20), ACCENT)
            && near(f.pixel(cx, cy), CARD)
    };
    wait_until("gpui to draw all three surfaces", || {
        drawn(&person.screenshot(0).unwrap())
    });
    shot(&person.screenshot(0).unwrap(), "desktop-gpui-clients.png");

    // Input from the person's seat reaches gpui: a click changes the window.
    while frames.try_recv().is_ok() {}
    person.click(cx as f64, cy as f64).unwrap();
    let after = wait_frame(&frames, "the click changes the gpui window", |f| {
        near(f.pixel(cx, cy), ACCENT)
    });
    shot(&after, "desktop-gpui-clicked.png");
}
