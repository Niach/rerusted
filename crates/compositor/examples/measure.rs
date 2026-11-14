//! First numbers for #RUST-20 on the headless backend.
//!
//! `cargo run -p compositor --release --example measure`
//!
//! Software rendering on whatever machine runs it: the numbers say what the
//! design costs, not what a GPU session will reach.

#[cfg(target_os = "linux")]
#[path = "../tests/common/probe.rs"]
mod probe;

#[cfg(not(target_os = "linux"))]
fn main() {}

#[cfg(target_os = "linux")]
fn main() {
    linux::main();
}

#[cfg(target_os = "linux")]
mod linux {
    use std::time::{Duration, Instant};

    use compositor::{Backend, Config, DeskHandle, Handle, SpaceConfig};

    use crate::probe::{Probe, ProbeConfig};

    const SIZE: (i32, i32) = (1280, 800);

    fn start(dir: &std::path::Path) -> Handle {
        let mut config = Config::new(Backend::Headless);
        config.runtime_dir = Some(dir.to_path_buf());
        config.name = "measure".into();
        compositor::start(config).unwrap()
    }

    fn space(handle: &Handle, xwayland: bool) -> DeskHandle {
        handle
            .create_space(SpaceConfig {
                size: SIZE,
                xwayland,
            })
            .unwrap()
    }

    fn wait_window(desk: &DeskHandle) {
        while !desk.windows().unwrap().iter().any(|w| w.geometry.w > 0) {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn percentiles(mut samples: Vec<Duration>) -> String {
        samples.sort();
        let at = |p: f64| samples[((samples.len() - 1) as f64 * p) as usize].as_secs_f64() * 1000.0;
        format!(
            "p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms (n={})",
            at(0.5),
            at(0.95),
            at(1.0),
            samples.len()
        )
    }

    /// Time from the injected action to the delivered frame that shows the change.
    fn latency(dir: &std::path::Path) {
        let handle = start(dir);

        let desk = space(&handle, false);
        let frames = desk.frames(0).unwrap();
        let _probe = Probe::spawn(ProbeConfig::new(desk.socket(), "latency"));
        wait_window(&desk);
        while frames.recv_timeout(Duration::from_millis(300)).is_ok() {}
        let run = |what: &str, act: &dyn Fn(), gap: Duration| {
            let mut samples = Vec::new();
            for _ in 0..200 {
                std::thread::sleep(gap);
                let sent = Instant::now();
                act();
                let frame = frames
                    .recv_timeout(Duration::from_secs(5))
                    .expect("a frame after the action");
                samples.push(frame.time.duration_since(sent));
            }
            println!("  {what}: {}", percentiles(samples));
        };
        let idle = Duration::from_millis(40);
        run(
            "click, whole 1280x800 window repaints, space was idle",
            &|| desk.click(300.0, 300.0).unwrap(),
            idle,
        );
        run(
            "key, a 16x16 mark changes, space was idle",
            &|| desk.type_text("x").unwrap(),
            idle,
        );
        run(
            "key, a 16x16 mark changes, back to back",
            &|| desk.type_text("x").unwrap(),
            Duration::ZERO,
        );
        desk.close().unwrap();

        if which("xterm") && which("Xwayland") {
            let desk = space(&handle, true);
            let frames = desk.frames(0).unwrap();
            let mut child = desk
                .command("xterm")
                .env_remove("WAYLAND_DISPLAY")
                .spawn()
                .unwrap();
            wait_window(&desk);
            desk.click(200.0, 200.0).unwrap();
            while frames.recv_timeout(Duration::from_millis(500)).is_ok() {}
            let mut samples = Vec::new();
            for _ in 0..100 {
                std::thread::sleep(idle);
                let sent = Instant::now();
                desk.type_text("x").unwrap();
                let frame = frames
                    .recv_timeout(Duration::from_secs(5))
                    .expect("xterm echoes the key");
                samples.push(frame.time.duration_since(sent));
                while frames.recv_timeout(Duration::from_millis(20)).is_ok() {}
            }
            println!(
                "  key echoed by xterm (X11, through the space's XWayland): {}",
                percentiles(samples)
            );
            let _ = child.kill();
            let _ = child.wait();
        }
        handle.stop();
    }

    fn which(program: &str) -> bool {
        std::env::var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|dir| dir.join(program).exists())
        })
    }

    /// CPU of the compositor thread and memory for `n` spaces.
    fn cost(dir: &std::path::Path, n: usize, busy: bool, watched: bool) {
        let handle = start(dir);
        let base = handle.stats().unwrap();
        let mut keep = Vec::new();
        for i in 0..n {
            let desk = space(&handle, false);
            let mut config = ProbeConfig::new(desk.socket(), &format!("space {i}"));
            config.busy = busy;
            let probe = Probe::spawn(config);
            wait_window(&desk);
            let frames = watched.then(|| desk.frames(0).unwrap());
            keep.push((desk, probe, frames));
        }
        std::thread::sleep(Duration::from_secs(1));
        // An agent that watches takes every frame as it comes.
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut readers = Vec::new();
        for (_, _, frames) in keep.iter_mut() {
            if let Some(frames) = frames.take() {
                let stop = stop.clone();
                readers.push(std::thread::spawn(move || {
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = frames.recv_timeout(Duration::from_millis(50));
                    }
                }));
            }
        }
        let before = handle.stats().unwrap();
        let window = Duration::from_secs(5);
        std::thread::sleep(window);
        let after = handle.stats().unwrap();
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        readers.into_iter().for_each(|r| r.join().unwrap());

        let cpu = (after.cpu - before.cpu).as_secs_f64() / window.as_secs_f64() * 100.0;
        let renders = (after.renders - before.renders) as f64 / window.as_secs_f64();
        let frames = (after.frames - before.frames) as f64 / window.as_secs_f64();
        let memory = (after.rss_bytes.saturating_sub(base.rss_bytes)) as f64 / n as f64 / 1e6;
        println!(
            "  {n:>2} {} {}: compositor thread {cpu:>5.1} % of one core, {renders:>6.1} renders/s, \
             {frames:>6.1} frames/s delivered, {memory:>5.1} MB per space (with its test client)",
            if busy { "busy" } else { "idle" },
            if watched { "watched  " } else { "unwatched" },
        );
        drop(keep);
        handle.stop();
        std::thread::sleep(Duration::from_millis(200));
    }

    /// Resident memory of the X server one space brings along.
    fn xwayland(dir: &std::path::Path) {
        if !which("Xwayland") {
            return;
        }
        let handle = start(dir);
        let _desk = space(&handle, true);
        std::thread::sleep(Duration::from_secs(1));
        let me = std::process::id().to_string();
        for entry in std::fs::read_dir("/proc").unwrap().flatten() {
            let status = std::fs::read_to_string(entry.path().join("status")).unwrap_or_default();
            let field = |name: &str| {
                status
                    .lines()
                    .find_map(|l| l.strip_prefix(name))
                    .map(|v| v.trim().to_string())
                    .unwrap_or_default()
            };
            if field("Name:") == "Xwayland" && field("PPid:") == me {
                println!("  one idle XWayland: {} resident", field("VmRSS:"));
            }
        }
        handle.stop();
    }

    pub fn main() {
        let dir = tempfile::tempdir().unwrap();
        println!(
            "spaces are {}x{}, headless backend (pixman, no GPU)",
            SIZE.0, SIZE.1
        );
        println!("action to observed change:");
        latency(dir.path());
        println!("cost of N spaces, over 5 s:");
        for n in [1, 4, 16] {
            cost(dir.path(), n, false, true);
        }
        for n in [1, 4, 16] {
            cost(dir.path(), n, true, false);
        }
        for n in [1, 4, 16] {
            cost(dir.path(), n, true, true);
        }
        xwayland(dir.path());
    }
}
