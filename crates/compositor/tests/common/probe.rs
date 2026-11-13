//! A small Wayland client for the tests. It runs on a thread of its own, opens
//! one window, records every input event it receives and repaints in ways the
//! tests can predict:
//!
//! - a pointer button press toggles the background colour (the whole window changes),
//! - every key press that produces text paints one 16x16 square (only that square changes).

#![allow(dead_code)]

use std::{
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    data_device_manager::{
        DataDeviceManagerState, WritePipe,
        data_device::{DataDevice, DataDeviceHandler},
        data_offer::{DataOfferHandler, DragOffer},
        data_source::{CopyPasteSource, DataSourceHandler},
    },
    delegate_compositor, delegate_data_device, delegate_keyboard, delegate_output,
    delegate_pointer, delegate_registry, delegate_seat, delegate_shm, delegate_xdg_shell,
    delegate_xdg_window,
    output::{OutputHandler, OutputState},
    reexports::{
        calloop::{EventLoop, channel},
        calloop_wayland_source::WaylandSource,
    },
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    seat::{
        Capability, SeatHandler, SeatState,
        keyboard::{KeyEvent, KeyboardHandler, Keysym, Modifiers},
        pointer::{PointerEvent, PointerEventKind, PointerHandler},
    },
    shell::{
        WaylandSurface,
        xdg::{
            XdgShell,
            window::{Window, WindowConfigure, WindowDecorations, WindowHandler},
        },
    },
    shm::{Shm, ShmHandler, slot::SlotPool},
};
use wayland_client::{
    Connection, QueueHandle,
    globals::registry_queue_init,
    protocol::{
        wl_data_device::WlDataDevice,
        wl_data_device_manager::DndAction,
        wl_data_source::WlDataSource,
        wl_keyboard::WlKeyboard,
        wl_output::{self, WlOutput},
        wl_pointer::WlPointer,
        wl_seat::WlSeat,
        wl_shm,
        wl_surface::WlSurface,
    },
};

pub const MIME: &str = "text/plain;charset=utf-8";
/// The two background colours, as (red, green, blue).
pub const BACKGROUNDS: [(u8, u8, u8); 2] = [(0x20, 0x40, 0x80), (0xc0, 0x50, 0x20)];
/// The colour of the squares painted for typed text.
pub const SQUARE: (u8, u8, u8) = (0xf0, 0xf0, 0xf0);

/// The rectangle of the n-th typed square, in window coordinates.
pub fn square(n: usize) -> (i32, i32, i32, i32) {
    (8 + 20 * (n as i32 % 30), 8 + 20 * (n as i32 / 30), 16, 16)
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    PointerEnter,
    PointerLeave,
    Motion(f64, f64),
    Button {
        x: f64,
        y: f64,
        button: u32,
        pressed: bool,
    },
    KeyboardEnter,
    KeyboardLeave,
    Key {
        text: Option<String>,
        pressed: bool,
    },
    Selection(Option<Vec<String>>),
}

#[derive(Debug, Clone, Default)]
pub struct Globals {
    pub seats: usize,
    pub outputs: usize,
    pub layer_shell: bool,
    pub seat_names: Vec<String>,
    pub output_names: Vec<String>,
}

#[derive(Clone)]
pub struct ProbeConfig {
    pub socket: PathBuf,
    pub title: String,
    pub size: (u32, u32),
    /// Repaint on every frame callback.
    pub busy: bool,
    /// Offer this text as the clipboard selection as soon as the keyboard enters.
    pub selection_on_focus: Option<String>,
    /// Offer this text as the clipboard selection on a pointer button press.
    pub selection_on_click: Option<String>,
}

impl ProbeConfig {
    pub fn new(socket: &std::path::Path, title: &str) -> Self {
        Self {
            socket: socket.to_path_buf(),
            title: title.into(),
            size: (640, 480),
            busy: false,
            selection_on_focus: None,
            selection_on_click: None,
        }
    }
}

enum Command {
    Paste(mpsc::Sender<Option<String>>),
    Stop,
}

pub struct Probe {
    events: Arc<Mutex<Vec<Event>>>,
    globals: Arc<Mutex<Globals>>,
    commands: channel::Sender<Command>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Probe {
    /// Starts the client and waits until its window is on screen.
    pub fn spawn(config: ProbeConfig) -> Probe {
        let events = Arc::new(Mutex::new(Vec::new()));
        let globals = Arc::new(Mutex::new(Globals::default()));
        let (commands, command_rx) = channel::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (events2, globals2) = (events.clone(), globals.clone());
        let thread = std::thread::Builder::new()
            .name(format!("probe {}", config.title))
            .spawn(move || run(config, events2, globals2, command_rx, ready_tx))
            .unwrap();
        ready_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the probe window did not appear");
        let probe = Probe {
            events,
            globals,
            commands,
            thread: Some(thread),
        };
        // A new window gets the keyboard of its desk. Until the client holds
        // it, typed keys have nowhere to go.
        probe.wait_for("the keyboard", |e| e.contains(&Event::KeyboardEnter));
        probe
    }

    pub fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    pub fn globals(&self) -> Globals {
        self.globals.lock().unwrap().clone()
    }

    /// All text typed into the window so far.
    pub fn typed(&self) -> String {
        self.events()
            .iter()
            .filter_map(|e| match e {
                Event::Key {
                    text: Some(t),
                    pressed: true,
                } => Some(t.clone()),
                _ => None,
            })
            .collect()
    }

    /// Waits until the recorded events satisfy `done`.
    pub fn wait_for(&self, what: &str, done: impl Fn(&[Event]) -> bool) {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(10) {
            if done(&self.events()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("timed out waiting for {what}; events: {:?}", self.events());
    }

    /// Reads the clipboard selection this client is offered.
    pub fn paste(&self) -> Option<String> {
        let (tx, rx) = mpsc::channel();
        self.commands.send(Command::Paste(tx)).unwrap();
        rx.recv_timeout(Duration::from_secs(10))
            .expect("paste did not answer")
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct App {
    registry_state: RegistryState,
    seat_state: SeatState,
    output_state: OutputState,
    shm: Shm,
    data_device_manager: DataDeviceManagerState,
    pool: SlotPool,
    window: Window,
    config: ProbeConfig,
    size: (u32, u32),
    events: Arc<Mutex<Vec<Event>>>,
    globals: Arc<Mutex<Globals>>,
    ready: Option<mpsc::Sender<()>>,
    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,
    data_device: Option<DataDevice>,
    sources: Vec<(CopyPasteSource, String)>,
    background: usize,
    squares: usize,
    tick: u32,
    pointer_at: (f64, f64),
    configured: bool,
    exit: bool,
}

fn run(
    config: ProbeConfig,
    events: Arc<Mutex<Vec<Event>>>,
    globals: Arc<Mutex<Globals>>,
    commands: channel::Channel<Command>,
    ready: mpsc::Sender<()>,
) {
    let stream = std::os::unix::net::UnixStream::connect(&config.socket)
        .expect("connect to the desk's socket");
    let conn = Connection::from_socket(stream).unwrap();
    let (global_list, queue) = registry_queue_init::<App>(&conn).unwrap();
    let qh = queue.handle();

    {
        let mut g = globals.lock().unwrap();
        global_list.contents().with_list(|list| {
            g.seats = list.iter().filter(|x| x.interface == "wl_seat").count();
            g.outputs = list.iter().filter(|x| x.interface == "wl_output").count();
            g.layer_shell = list.iter().any(|x| x.interface == "zwlr_layer_shell_v1");
        });
    }

    let mut event_loop: EventLoop<App> = EventLoop::try_new().unwrap();
    WaylandSource::new(conn.clone(), queue)
        .insert(event_loop.handle())
        .unwrap();

    let compositor = CompositorState::bind(&global_list, &qh).unwrap();
    let xdg_shell = XdgShell::bind(&global_list, &qh).unwrap();
    let shm = Shm::bind(&global_list, &qh).unwrap();
    let data_device_manager = DataDeviceManagerState::bind(&global_list, &qh).unwrap();

    let surface = compositor.create_surface(&qh);
    let window = xdg_shell.create_window(surface, WindowDecorations::RequestClient, &qh);
    window.set_title(config.title.clone());
    window.set_app_id("probe");
    window.commit();

    let pool = SlotPool::new((config.size.0 * config.size.1 * 4) as usize, &shm).unwrap();
    let mut app = App {
        registry_state: RegistryState::new(&global_list),
        seat_state: SeatState::new(&global_list, &qh),
        output_state: OutputState::new(&global_list, &qh),
        shm,
        data_device_manager,
        pool,
        window,
        size: config.size,
        config,
        events,
        globals,
        ready: Some(ready),
        keyboard: None,
        pointer: None,
        data_device: None,
        sources: Vec::new(),
        background: 0,
        squares: 0,
        tick: 0,
        pointer_at: (0.0, 0.0),
        configured: false,
        exit: false,
    };

    let conn2 = conn.clone();
    event_loop
        .handle()
        .insert_source(commands, move |event, _, app| match event {
            channel::Event::Msg(Command::Paste(reply)) => app.paste(&conn2, reply),
            channel::Event::Msg(Command::Stop) | channel::Event::Closed => app.exit = true,
        })
        .unwrap();

    while !app.exit {
        if event_loop
            .dispatch(Duration::from_millis(100), &mut app)
            .is_err()
        {
            break;
        }
    }
}

impl App {
    fn record(&self, event: Event) {
        self.events.lock().unwrap().push(event);
    }

    /// Paints the whole window but reports only `damage` as changed.
    fn draw(&mut self, qh: &QueueHandle<Self>, damage: (i32, i32, i32, i32)) {
        let (w, h) = self.size;
        let Ok((buffer, canvas)) =
            self.pool
                .create_buffer(w as i32, h as i32, w as i32 * 4, wl_shm::Format::Xrgb8888)
        else {
            return;
        };
        let bg = BACKGROUNDS[self.background % 2];
        for pixel in canvas.as_chunks_mut::<4>().0 {
            pixel.copy_from_slice(&[bg.2, bg.1, bg.0, 0xff]);
        }
        let mut fill = |rect: (i32, i32, i32, i32), colour: (u8, u8, u8)| {
            for y in rect.1.max(0)..(rect.1 + rect.3).min(h as i32) {
                for x in rect.0.max(0)..(rect.0 + rect.2).min(w as i32) {
                    let i = ((y as u32 * w + x as u32) * 4) as usize;
                    canvas[i..i + 4].copy_from_slice(&[colour.2, colour.1, colour.0, 0xff]);
                }
            }
        };
        for n in 0..self.squares {
            fill(square(n), SQUARE);
        }
        if self.config.busy {
            let shade = (self.tick % 256) as u8;
            fill((0, h as i32 - 64, 256, 64), (shade, 255 - shade, 0x80));
        }
        let surface = self.window.wl_surface();
        surface.damage_buffer(damage.0, damage.1, damage.2, damage.3);
        if self.config.busy {
            surface.frame(qh, surface.clone());
        }
        let _ = buffer.attach_to(surface);
        self.window.commit();
    }

    fn offer(&mut self, qh: &QueueHandle<Self>, text: String, serial: u32) {
        let Some(device) = &self.data_device else {
            return;
        };
        let source = self
            .data_device_manager
            .create_copy_paste_source(qh, [MIME]);
        source.set_selection(device, serial);
        self.sources.push((source, text));
    }

    fn paste(&mut self, conn: &Connection, reply: mpsc::Sender<Option<String>>) {
        let offer = self
            .data_device
            .as_ref()
            .and_then(|d| d.data().selection_offer());
        let Some(offer) = offer else {
            let _ = reply.send(None);
            return;
        };
        match offer.receive(MIME.to_string()) {
            Ok(mut pipe) => {
                let _ = conn.flush();
                // The owner of the selection may be this very client, so the
                // read must not block the thread that would answer it.
                std::thread::spawn(move || {
                    let mut text = String::new();
                    let _ = pipe.read_to_string(&mut text);
                    let _ = reply.send(Some(text));
                });
            }
            Err(_) => {
                let _ = reply.send(None);
            }
        }
    }
}

impl CompositorHandler for App {
    fn scale_factor_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: i32,
    ) {
    }
    fn transform_changed(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: wl_output::Transform,
    ) {
    }
    fn frame(&mut self, _: &Connection, qh: &QueueHandle<Self>, _: &WlSurface, _: u32) {
        if self.config.busy {
            self.tick += 1;
            let h = self.size.1 as i32;
            self.draw(qh, (0, h - 64, 256, 64));
        }
    }
    fn surface_enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &WlOutput,
    ) {
    }
    fn surface_leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlSurface,
        _: &WlOutput,
    ) {
    }
}

impl OutputHandler for App {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, conn: &Connection, qh: &QueueHandle<Self>, output: WlOutput) {
        self.update_output(conn, qh, output);
    }
    fn update_output(&mut self, _: &Connection, _: &QueueHandle<Self>, output: WlOutput) {
        if let Some(name) = self.output_state.info(&output).and_then(|i| i.name) {
            let mut g = self.globals.lock().unwrap();
            if !g.output_names.contains(&name) {
                g.output_names.push(name);
            }
        }
    }
    fn output_destroyed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlOutput) {}
}

impl WindowHandler for App {
    fn request_close(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &Window) {
        self.exit = true;
    }

    fn configure(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &Window,
        configure: WindowConfigure,
        _: u32,
    ) {
        let size = (
            configure
                .new_size
                .0
                .map(|v| v.get())
                .unwrap_or(self.config.size.0),
            configure
                .new_size
                .1
                .map(|v| v.get())
                .unwrap_or(self.config.size.1),
        );
        if self.configured && size == self.size {
            return;
        }
        self.size = size;
        self.configured = true;
        self.draw(qh, (0, 0, size.0 as i32, size.1 as i32));
        if let Some(ready) = self.ready.take() {
            let _ = ready.send(());
        }
    }
}

impl SeatHandler for App {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }
    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if let Some(name) = self.seat_state.info(&seat).and_then(|i| i.name) {
            let mut g = self.globals.lock().unwrap();
            if !g.seat_names.contains(&name) {
                g.seat_names.push(name);
            }
        }
        if capability == Capability::Keyboard && self.keyboard.is_none() {
            self.keyboard = self.seat_state.get_keyboard(qh, &seat, None).ok();
        }
        if capability == Capability::Pointer && self.pointer.is_none() {
            self.pointer = self.seat_state.get_pointer(qh, &seat).ok();
        }
        if self.data_device.is_none() {
            self.data_device = Some(self.data_device_manager.get_data_device(qh, &seat));
        }
    }
    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: WlSeat,
        _: Capability,
    ) {
    }
    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, _: WlSeat) {}
}

impl KeyboardHandler for App {
    fn enter(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &WlSurface,
        serial: u32,
        _: &[u32],
        _: &[Keysym],
    ) {
        self.record(Event::KeyboardEnter);
        if let Some(text) = self.config.selection_on_focus.take() {
            self.offer(qh, text, serial);
        }
    }
    fn leave(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: &WlSurface,
        _: u32,
    ) {
        self.record(Event::KeyboardLeave);
    }
    fn press_key(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        event: KeyEvent,
    ) {
        let text = match event.keysym {
            Keysym::Return => Some("\n".to_string()),
            _ => event.utf8.clone(),
        };
        let paints = text.is_some();
        self.record(Event::Key {
            text,
            pressed: true,
        });
        if paints {
            self.squares += 1;
            self.draw(qh, square(self.squares - 1));
        }
    }
    fn release_key(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        _: KeyEvent,
    ) {
    }
    fn update_modifiers(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlKeyboard,
        _: u32,
        _: Modifiers,
        _: u32,
    ) {
    }
}

impl PointerHandler for App {
    fn pointer_frame(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        _: &WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            match event.kind {
                PointerEventKind::Enter { .. } => {
                    self.pointer_at = event.position;
                    self.record(Event::PointerEnter);
                }
                PointerEventKind::Leave { .. } => self.record(Event::PointerLeave),
                PointerEventKind::Motion { .. } => {
                    self.pointer_at = event.position;
                    self.record(Event::Motion(event.position.0, event.position.1));
                }
                PointerEventKind::Press { button, serial, .. } => {
                    self.record(Event::Button {
                        x: event.position.0,
                        y: event.position.1,
                        button,
                        pressed: true,
                    });
                    if let Some(text) = self.config.selection_on_click.clone() {
                        self.offer(qh, text, serial);
                    }
                    self.background += 1;
                    self.draw(qh, (0, 0, self.size.0 as i32, self.size.1 as i32));
                }
                PointerEventKind::Release { button, .. } => {
                    self.record(Event::Button {
                        x: event.position.0,
                        y: event.position.1,
                        button,
                        pressed: false,
                    });
                }
                PointerEventKind::Axis { .. } => {}
            }
        }
    }
}

impl ShmHandler for App {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl DataDeviceHandler for App {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _: f64,
        _: f64,
        _: &WlSurface,
    ) {
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {
        let mimes = self
            .data_device
            .as_ref()
            .and_then(|d| d.data().selection_offer())
            .map(|o| o.with_mime_types(|m| m.to_vec()));
        self.record(Event::Selection(mimes));
    }
    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}

impl DataOfferHandler for App {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

impl DataSourceHandler for App {
    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        _: String,
        mut fd: WritePipe,
    ) {
        if let Some((_, text)) = self.sources.iter().find(|(s, _)| s.inner() == source) {
            let _ = fd.write_all(text.as_bytes());
        }
    }
    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        self.sources.retain(|(s, _)| s.inner() != source);
    }
    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

delegate_compositor!(App);
delegate_output!(App);
delegate_shm!(App);
delegate_seat!(App);
delegate_keyboard!(App);
delegate_pointer!(App);
delegate_xdg_shell!(App);
delegate_xdg_window!(App);
delegate_data_device!(App);
delegate_registry!(App);

impl ProvidesRegistryState for App {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
}
