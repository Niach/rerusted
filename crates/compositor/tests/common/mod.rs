#![allow(dead_code)]

pub mod probe;

use std::{
    sync::mpsc::Receiver,
    time::{Duration, Instant},
};

use compositor::{Backend, Config, DeskHandle, Frame, Handle};

/// A headless compositor with its sockets in a directory of its own.
pub struct Rig {
    pub handle: Handle,
    pub dir: tempfile::TempDir,
}

impl Rig {
    pub fn start() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::new(Backend::Headless);
        config.runtime_dir = Some(dir.path().to_path_buf());
        config.name = "test".into();
        Rig {
            handle: compositor::start(config).unwrap(),
            dir,
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.handle.stop();
    }
}

/// Waits for the first frame that satisfies `wanted`.
pub fn wait_frame(frames: &Receiver<Frame>, what: &str, wanted: impl Fn(&Frame) -> bool) -> Frame {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match frames.recv_timeout(left) {
            Ok(frame) if wanted(&frame) => return frame,
            Ok(_) => {}
            Err(_) => panic!("timed out waiting for a frame: {what}"),
        }
    }
}

/// Drops every frame that is already waiting or arrives within `quiet`.
pub fn drain(frames: &Receiver<Frame>, quiet: Duration) -> usize {
    let mut n = 0;
    while frames.recv_timeout(quiet).is_ok() {
        n += 1;
    }
    n
}

pub fn wait_until(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}");
}

/// Waits until a desk shows a window with content, as an agent would before it acts.
pub fn wait_window(desk: &DeskHandle) {
    wait_until("a window with content", || {
        desk.windows().unwrap().iter().any(|w| w.geometry.w > 0)
    });
}
