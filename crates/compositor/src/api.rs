//! The public surface: configuration, the control handle and the data it returns.

use std::{
    fmt,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use smithay::reexports::calloop::channel::Sender;

use crate::state::State;

/// Where the outputs end up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Rendered in memory with pixman. No screen, no GPU.
    Headless,
    /// Shown in one window on the host desktop (winit, GLES).
    Nested,
}

/// One output of the person's desk.
#[derive(Debug, Clone)]
pub struct OutputConfig {
    pub name: String,
    pub size: (i32, i32),
}

/// What to start.
#[derive(Debug, Clone)]
pub struct Config {
    pub backend: Backend,
    /// The person's outputs, laid out left to right.
    pub outputs: Vec<OutputConfig>,
    /// Directory for the Wayland sockets. Defaults to `XDG_RUNTIME_DIR`.
    pub runtime_dir: Option<PathBuf>,
    /// Prefix of the socket names.
    pub name: String,
}

impl Config {
    /// Two outputs of 960x600 on the given backend.
    pub fn new(backend: Backend) -> Self {
        Self {
            backend,
            outputs: vec![
                OutputConfig {
                    name: "out-1".into(),
                    size: (960, 600),
                },
                OutputConfig {
                    name: "out-2".into(),
                    size: (960, 600),
                },
            ],
            runtime_dir: None,
            name: format!("rerusted-{}", std::process::id()),
        }
    }
}

/// An agent space to create.
#[derive(Debug, Clone)]
pub struct SpaceConfig {
    pub size: (i32, i32),
    /// Start an XWayland instance of its own for this space.
    pub xwayland: bool,
}

impl Default for SpaceConfig {
    fn default() -> Self {
        Self {
            size: (800, 600),
            xwayland: true,
        }
    }
}

/// A seat together with the outputs it works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeskId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeskKind {
    /// The person's desk: real (or nested, or headless) outputs.
    Person,
    /// An agent space: one virtual output that is never scanned out.
    Space,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputKind {
    Nested,
    Headless,
    Virtual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputInfo {
    pub name: String,
    pub desk: DeskId,
    pub kind: OutputKind,
    /// Position and size inside its desk.
    pub geometry: Rect,
    /// What is left after panels reserved their exclusive zones, relative to the output.
    pub usable: Rect,
    /// Layer-shell surfaces mapped on this output: (namespace, layer).
    pub layers: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatInfo {
    pub name: String,
    pub desk: DeskId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub title: String,
    pub app_id: String,
    pub x11: bool,
    pub geometry: Rect,
}

/// Everything about a seat that another seat must never change.
#[derive(Debug, Clone, PartialEq)]
pub struct SeatSnapshot {
    pub pointer: (f64, f64),
    pub pointer_focus: Option<String>,
    pub keyboard_focus: Option<String>,
    /// Mime types of the clipboard selection held on this seat.
    pub selection: Option<Vec<String>>,
    /// Input events the compositor has sent through this seat so far.
    pub input_events: u64,
}

/// The content of one output after a change.
#[derive(Clone)]
pub struct Frame {
    /// Counts the frames delivered for this output.
    pub serial: u64,
    /// When the compositor finished reading the frame back.
    pub time: Instant,
    pub size: (u32, u32),
    /// What changed since the previous frame.
    pub damage: Vec<Rect>,
    /// `size.0 * size.1 * 4` bytes, red, green, blue, alpha per pixel.
    pub pixels: Arc<Vec<u8>>,
}

impl fmt::Debug for Frame {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Frame")
            .field("serial", &self.serial)
            .field("size", &self.size)
            .field("damage", &self.damage)
            .finish_non_exhaustive()
    }
}

impl Frame {
    /// The colour at a pixel as (red, green, blue).
    pub fn pixel(&self, x: u32, y: u32) -> (u8, u8, u8) {
        let i = ((y * self.size.0 + x) * 4) as usize;
        (self.pixels[i], self.pixels[i + 1], self.pixels[i + 2])
    }

    /// The smallest rectangle around all damage.
    pub fn damage_bounds(&self) -> Rect {
        let mut it = self.damage.iter();
        let Some(first) = it.next() else {
            return Rect::default();
        };
        let (mut x0, mut y0, mut x1, mut y1) =
            (first.x, first.y, first.x + first.w, first.y + first.h);
        for r in it {
            x0 = x0.min(r.x);
            y0 = y0.min(r.y);
            x1 = x1.max(r.x + r.w);
            y1 = y1.max(r.y + r.h);
        }
        Rect {
            x: x0,
            y: y0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }

    pub fn write_png(&self, path: &Path) -> Result<(), Error> {
        let file =
            std::fs::File::create(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let mut encoder =
            png::Encoder::new(std::io::BufWriter::new(file), self.size.0, self.size.1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut opaque = self.pixels.as_ref().clone();
        opaque
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .for_each(|p| p[3] = 255);
        let mut writer = encoder.write_header().map_err(|e| Error(e.to_string()))?;
        writer
            .write_image_data(&opaque)
            .map_err(|e| Error(e.to_string()))
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    /// CPU time the compositor thread has used.
    pub cpu: Duration,
    /// Resident memory of the whole process.
    pub rss_bytes: u64,
    /// Output renders that ran (with or without damage).
    pub renders: u64,
    /// Frames handed to subscribers.
    pub frames: u64,
}

#[derive(Debug, Clone)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub(crate) type Job = Box<dyn FnOnce(&mut State) + Send>;

/// Control of a running compositor from any thread.
#[derive(Clone)]
pub struct Handle {
    pub(crate) tx: Sender<Job>,
    pub(crate) person: DeskHandle,
}

impl Handle {
    pub(crate) fn call<R: Send + 'static>(
        tx: &Sender<Job>,
        f: impl FnOnce(&mut State) -> R + Send + 'static,
    ) -> Result<R, Error> {
        let (reply, rx) = mpsc::channel();
        tx.send(Box::new(move |state| {
            let _ = reply.send(f(state));
        }))
        .map_err(|_| Error("the compositor has stopped".into()))?;
        rx.recv()
            .map_err(|_| Error("the compositor has stopped".into()))
    }

    /// The person's desk.
    pub fn person(&self) -> &DeskHandle {
        &self.person
    }

    /// Creates an agent space. Returns once its XWayland (if any) is ready.
    pub fn create_space(&self, config: SpaceConfig) -> Result<DeskHandle, Error> {
        let (reply, rx) = mpsc::channel();
        self.tx
            .send(Box::new(move |state| state.create_space(config, reply)))
            .map_err(|_| Error("the compositor has stopped".into()))?;
        let info = rx
            .recv_timeout(Duration::from_secs(20))
            .map_err(|_| Error("the space did not become ready".into()))??;
        Ok(DeskHandle {
            tx: self.tx.clone(),
            id: info.0,
            kind: DeskKind::Space,
            socket: info.1,
            x11_display: info.2,
        })
    }

    pub fn outputs(&self) -> Result<Vec<OutputInfo>, Error> {
        Self::call(&self.tx, |s| s.output_infos())
    }

    pub fn seats(&self) -> Result<Vec<SeatInfo>, Error> {
        Self::call(&self.tx, |s| s.seat_infos())
    }

    pub fn stats(&self) -> Result<Stats, Error> {
        Self::call(&self.tx, |s| s.stats())
    }

    /// Stops the compositor.
    pub fn stop(&self) {
        let _ = self.tx.send(Box::new(|s| s.loop_signal.stop()));
    }
}

/// Control of one desk: the person's or an agent space.
#[derive(Clone)]
pub struct DeskHandle {
    pub(crate) tx: Sender<Job>,
    pub(crate) id: DeskId,
    pub(crate) kind: DeskKind,
    pub(crate) socket: PathBuf,
    pub(crate) x11_display: Option<u32>,
}

impl DeskHandle {
    pub fn id(&self) -> DeskId {
        self.id
    }

    pub fn kind(&self) -> DeskKind {
        self.kind
    }

    /// The Wayland socket of this desk. A client that connects here belongs to this desk.
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// The X11 display number of this desk's XWayland.
    pub fn x11_display(&self) -> Option<u32> {
        self.x11_display
    }

    /// A command that will open its windows on this desk.
    pub fn command(&self, program: &str) -> Command {
        let mut command = Command::new(program);
        command.env("WAYLAND_DISPLAY", &self.socket);
        match self.x11_display {
            Some(n) => command.env("DISPLAY", format!(":{n}")),
            None => command.env_remove("DISPLAY"),
        };
        command
    }

    pub fn pointer_move(&self, x: f64, y: f64) -> Result<(), Error> {
        let id = self.id;
        Handle::call(&self.tx, move |s| s.pointer_motion(id, (x, y).into()))
    }

    /// `button` is a Linux button code, see [`crate::keys`].
    pub fn button(&self, button: u32, pressed: bool) -> Result<(), Error> {
        let id = self.id;
        Handle::call(&self.tx, move |s| s.pointer_button(id, button, pressed))
    }

    /// Moves the pointer and presses and releases the left button.
    pub fn click(&self, x: f64, y: f64) -> Result<(), Error> {
        let id = self.id;
        Handle::call(&self.tx, move |s| {
            s.pointer_motion(id, (x, y).into());
            s.pointer_button(id, crate::keys::BTN_LEFT, true);
            s.pointer_button(id, crate::keys::BTN_LEFT, false);
        })
    }

    /// `key` is a Linux key code, see [`crate::keys`].
    pub fn key(&self, key: u32, pressed: bool) -> Result<(), Error> {
        let id = self.id;
        Handle::call(&self.tx, move |s| s.key(id, key, pressed))
    }

    /// Types text on a US layout. `\n` is Enter.
    pub fn type_text(&self, text: &str) -> Result<(), Error> {
        let mut strokes = Vec::new();
        for c in text.chars() {
            strokes
                .push(crate::input::stroke(c).ok_or_else(|| Error(format!("no key for {c:?}")))?);
        }
        let id = self.id;
        Handle::call(&self.tx, move |s| {
            for (key, shift) in strokes {
                if shift {
                    s.key(id, crate::keys::KEY_LEFTSHIFT, true);
                }
                s.key(id, key, true);
                s.key(id, key, false);
                if shift {
                    s.key(id, crate::keys::KEY_LEFTSHIFT, false);
                }
            }
        })
    }

    pub fn windows(&self) -> Result<Vec<WindowInfo>, Error> {
        let id = self.id;
        Handle::call(&self.tx, move |s| s.window_infos(id))
    }

    pub fn seat_snapshot(&self) -> Result<SeatSnapshot, Error> {
        let id = self.id;
        Handle::call(&self.tx, move |s| s.seat_snapshot(id))?
            .ok_or_else(|| Error("no such desk".into()))
    }

    /// Frames of one output of this desk, delivered only when something changed.
    pub fn frames(&self, output: usize) -> Result<mpsc::Receiver<Frame>, Error> {
        let (tx, rx) = mpsc::channel();
        let id = self.id;
        Handle::call(&self.tx, move |s| s.subscribe(id, output, tx))?;
        Ok(rx)
    }

    /// The current content of one output, changed or not.
    pub fn screenshot(&self, output: usize) -> Result<Frame, Error> {
        let id = self.id;
        Handle::call(&self.tx, move |s| s.screenshot(id, output))?
    }

    /// Closes the desk: its windows, its XWayland, its socket, its seat and its outputs.
    pub fn close(&self) -> Result<(), Error> {
        let id = self.id;
        Handle::call(&self.tx, move |s| s.close_desk(id))
    }
}

/// Starts a headless compositor on its own thread.
pub fn start(config: Config) -> Result<Handle, Error> {
    if config.backend != Backend::Headless {
        return Err(Error(
            "start() is for the headless backend; the nested one needs the main thread, use run()"
                .into(),
        ));
    }
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("compositor".into())
        .spawn(move || {
            let on_ready = {
                let tx = tx.clone();
                move |handle: Handle| {
                    let _ = tx.send(Ok(handle));
                }
            };
            if let Err(e) = crate::state::run(config, on_ready) {
                let _ = tx.send(Err(e));
            }
        })
        .map_err(|e| Error(e.to_string()))?;
    rx.recv()
        .map_err(|_| Error("the compositor thread died".into()))?
}
