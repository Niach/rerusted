//! The compositor's state: desks, their outputs and seats, and the event loop.

use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

use smithay::{
    backend::renderer::damage::OutputDamageTracker,
    desktop::{PopupManager, Space, Window, layer_map_for_output},
    input::{Seat, SeatState, keyboard::XkbConfig},
    output::{Mode as OutputMode, Output, PhysicalProperties, Scale, Subpixel},
    reexports::{
        calloop::{
            EventLoop, Interest, LoopHandle, LoopSignal, Mode, PostAction, RegistrationToken,
            channel::{self, Event as ChannelEvent},
            generic::Generic,
            timer::{TimeoutAction, Timer},
        },
        wayland_server::{
            Client, Display, DisplayHandle, ListeningSocket, Resource,
            backend::{ClientData, ClientId, DisconnectReason, GlobalId},
            protocol::wl_surface::WlSurface,
        },
    },
    utils::{IsAlive, Transform},
    wayland::{
        compositor::{CompositorClientState, CompositorState, with_states},
        output::OutputManagerState,
        seat::WaylandFocus,
        selection::data_device::DataDeviceState,
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{XdgShellState, XdgToplevelSurfaceData},
        },
        shm::ShmState,
        viewporter::ViewporterState,
        xwayland_shell::XWaylandShellState,
    },
    xwayland::{X11Wm, XWayland, XWaylandClientData, XWaylandEvent},
};

use crate::{
    api::{
        Backend, Config, DeskHandle, DeskId, DeskKind, Error, Frame, Handle, Job, OutputInfo,
        OutputKind, Rect, SeatInfo, SeatSnapshot, SpaceConfig, Stats, WindowInfo,
    },
    render::{Gfx, Read, Target},
};

/// The shortest time between two renders of one output.
const FRAME: Duration = Duration::from_micros(16_667);

/// The person's desk always has this id.
pub(crate) const PERSON: DeskId = DeskId(0);

pub(crate) struct ClientState {
    pub compositor_state: CompositorClientState,
    pub desk: DeskId,
}

impl ClientData for ClientState {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

/// Marks an XWayland client with the desk it serves.
pub(crate) struct DeskTag(pub DeskId);

/// The desk a client belongs to, decided by the socket it connected to.
pub(crate) fn desk_of(client: &Client) -> Option<DeskId> {
    if let Some(state) = client.get_data::<ClientState>() {
        return Some(state.desk);
    }
    client
        .get_data::<XWaylandClientData>()?
        .user_data()
        .get::<DeskTag>()
        .map(|t| t.0)
}

pub(crate) struct OutputState {
    pub output: Output,
    pub global: GlobalId,
    pub kind: OutputKind,
    pub tracker: OutputDamageTracker,
    pub target: Option<Target>,
    pub scheduled: bool,
    pub last_render: Option<Instant>,
    pub subscribers: Vec<mpsc::Sender<Frame>>,
    pub serial: u64,
}

pub(crate) struct Desk {
    pub kind: DeskKind,
    pub seat: Seat<State>,
    pub seat_name: String,
    pub space: Space<Window>,
    pub outputs: Vec<OutputState>,
    pub socket: PathBuf,
    socket_token: RegistrationToken,
    pub clients: Vec<Client>,
    pub xwm: Option<X11Wm>,
    pub x_client: Option<Client>,
    pub selection: Option<Vec<String>>,
    pub input_events: u64,
    pub cascade: i32,
}

pub(crate) type SpaceReply = mpsc::Sender<Result<(DeskId, PathBuf, Option<u32>), Error>>;

pub struct State {
    pub(crate) start: Instant,
    pub(crate) dh: DisplayHandle,
    pub(crate) loop_handle: LoopHandle<'static, State>,
    pub(crate) loop_signal: LoopSignal,

    pub(crate) compositor_state: CompositorState,
    pub(crate) xdg_shell_state: XdgShellState,
    pub(crate) shm_state: ShmState,
    pub(crate) seat_state: SeatState<State>,
    pub(crate) data_device_state: DataDeviceState,
    pub(crate) layer_shell_state: WlrLayerShellState,
    pub(crate) xwayland_shell_state: XWaylandShellState,
    pub(crate) popups: PopupManager,
    _output_manager_state: OutputManagerState,
    _viewporter_state: ViewporterState,

    pub(crate) desks: BTreeMap<DeskId, Desk>,
    /// Window managers of closed desks, kept until their X server is gone.
    pub(crate) closing_xwms: Vec<X11Wm>,
    next_desk: u32,
    pub(crate) gfx: Gfx,
    backend: Backend,
    runtime_dir: PathBuf,
    name: String,
    renders: u64,
    frames: u64,
    pub(crate) host_dirty: bool,
}

impl State {
    fn create_desk(
        &mut self,
        kind: DeskKind,
        outputs: &[(String, (i32, i32))],
    ) -> Result<DeskId, Error> {
        let id = DeskId(self.next_desk);
        self.next_desk += 1;
        let label = match kind {
            DeskKind::Person => "person".to_string(),
            DeskKind::Space => format!("space-{}", id.0),
        };

        let seat_name = format!("seat-{label}");
        let mut seat =
            self.seat_state
                .new_wl_seat_with_filter(&self.dh, seat_name.clone(), move |c| {
                    desk_of(c) == Some(id)
                });
        let xkb = XkbConfig {
            layout: "us",
            ..XkbConfig::default()
        };
        seat.add_keyboard(xkb, 200, 25)
            .map_err(|e| Error(format!("keyboard: {e:?}")))?;
        seat.add_pointer();

        let mut space = Space::default();
        let mut states = Vec::new();
        let mut x = 0;
        for (name, size) in outputs {
            let output_kind = match (kind, self.backend) {
                (DeskKind::Space, _) => OutputKind::Virtual,
                (DeskKind::Person, Backend::Headless) => OutputKind::Headless,
                (DeskKind::Person, Backend::Nested) => OutputKind::Nested,
            };
            let output = Output::new(
                name.clone(),
                PhysicalProperties {
                    size: (0, 0).into(),
                    subpixel: Subpixel::Unknown,
                    make: "rerusted".into(),
                    model: format!("{output_kind:?}"),
                },
            );
            let mode = OutputMode {
                size: (*size).into(),
                refresh: 60_000,
            };
            output.change_current_state(
                Some(mode),
                Some(Transform::Normal),
                Some(Scale::Integer(1)),
                Some((x, 0).into()),
            );
            output.set_preferred(mode);
            let global = output
                .create_global_with_filter::<State, _>(&self.dh, move |c| desk_of(c) == Some(id));
            space.map_output(&output, (x, 0));
            x += size.0;
            states.push(OutputState {
                tracker: OutputDamageTracker::from_output(&output),
                output,
                global,
                kind: output_kind,
                target: None,
                scheduled: false,
                last_render: None,
                subscribers: Vec::new(),
                serial: 0,
            });
        }

        let socket = self.runtime_dir.join(format!("{}-{label}", self.name));
        let listener = ListeningSocket::bind_absolute(socket.clone())
            .map_err(|e| Error(format!("{}: {e}", socket.display())))?;
        let socket_token = self
            .loop_handle
            .insert_source(
                Generic::new(listener, Interest::READ, Mode::Level),
                move |_, listener, state| {
                    while let Some(stream) = listener.accept()? {
                        let data = ClientState {
                            compositor_state: CompositorClientState::default(),
                            desk: id,
                        };
                        if let Ok(client) = state.dh.insert_client(stream, Arc::new(data))
                            && let Some(desk) = state.desks.get_mut(&id)
                        {
                            desk.clients.push(client);
                        }
                    }
                    Ok(PostAction::Continue)
                },
            )
            .map_err(|e| Error(e.to_string()))?;

        self.desks.insert(
            id,
            Desk {
                kind,
                seat,
                seat_name,
                space,
                outputs: states,
                socket,
                socket_token,
                clients: Vec::new(),
                xwm: None,
                x_client: None,
                selection: None,
                input_events: 0,
                cascade: 0,
            },
        );
        self.schedule(id);
        Ok(id)
    }

    pub(crate) fn create_space(&mut self, config: SpaceConfig, reply: SpaceReply) {
        let n = self.next_desk;
        let id = match self.create_desk(DeskKind::Space, &[(format!("space-{n}"), config.size)]) {
            Ok(id) => id,
            Err(e) => {
                let _ = reply.send(Err(e));
                return;
            }
        };
        let socket = self.desks[&id].socket.clone();
        if !config.xwayland {
            let _ = reply.send(Ok((id, socket, None)));
            return;
        }
        // One X server per space: X has a single core pointer per server, so
        // sharing one would let an agent move the pointer of every X client.
        let spawned = XWayland::spawn(
            &self.dh,
            None,
            std::iter::empty::<(String, String)>(),
            false,
            std::process::Stdio::null(),
            std::process::Stdio::null(),
            move |data| {
                data.insert_if_missing(|| DeskTag(id));
            },
        );
        let (xwayland, client) = match spawned {
            Ok(pair) => pair,
            Err(e) => {
                self.close_desk(id);
                let _ = reply.send(Err(Error(format!("Xwayland: {e}"))));
                return;
            }
        };
        if let Some(desk) = self.desks.get_mut(&id) {
            desk.x_client = Some(client.clone());
        }
        let inserted =
            self.loop_handle
                .insert_source(xwayland, move |event, _, state| match event {
                    XWaylandEvent::Ready {
                        x11_socket,
                        display_number,
                    } => {
                        match X11Wm::start_wm(state.loop_handle.clone(), x11_socket, client.clone())
                        {
                            Ok(wm) => {
                                if let Some(desk) = state.desks.get_mut(&id) {
                                    desk.xwm = Some(wm);
                                }
                                let _ = reply.send(Ok((id, socket.clone(), Some(display_number))));
                            }
                            Err(e) => {
                                state.close_desk(id);
                                let _ = reply.send(Err(Error(format!("X11 window manager: {e}"))));
                            }
                        }
                    }
                    XWaylandEvent::Error => {
                        state.close_desk(id);
                        let _ = reply.send(Err(Error("Xwayland failed to start".into())));
                    }
                });
        if let Err(e) = inserted {
            tracing::error!("XWayland source: {e}");
        }
    }

    pub(crate) fn close_desk(&mut self, id: DeskId) {
        if id == PERSON {
            return;
        }
        let Some(mut desk) = self.desks.remove(&id) else {
            return;
        };
        self.closing_xwms.extend(desk.xwm.take());
        self.loop_handle.remove(desk.socket_token);
        let _ = std::fs::remove_file(&desk.socket);
        let _ = std::fs::remove_file(desk.socket.with_extension("lock"));
        for client in desk.clients.iter().chain(desk.x_client.iter()) {
            self.dh
                .backend_handle()
                .kill_client(client.id(), DisconnectReason::ConnectionClosed);
        }
        for out in &desk.outputs {
            self.dh.remove_global::<State>(out.global.clone());
        }
        if let Some(global) = desk.seat.global() {
            self.dh.remove_global::<State>(global);
        }
        self.host_dirty = true;
    }

    pub(crate) fn desk_of_surface(&self, surface: &WlSurface) -> Option<DeskId> {
        self.dh
            .get_client(surface.id())
            .ok()
            .as_ref()
            .and_then(desk_of)
    }

    /// Asks for a render of every output of a desk, at most once per frame time.
    pub(crate) fn schedule(&mut self, id: DeskId) {
        let Some(desk) = self.desks.get_mut(&id) else {
            return;
        };
        for (index, out) in desk.outputs.iter_mut().enumerate() {
            if out.scheduled {
                continue;
            }
            out.scheduled = true;
            let wait = out
                .last_render
                .map(|t| FRAME.saturating_sub(t.elapsed()))
                .unwrap_or_default();
            let timer = Timer::from_duration(wait);
            let _ = self.loop_handle.insert_source(timer, move |_, _, state| {
                state.render(id, index, Read::OnDamage);
                TimeoutAction::Drop
            });
        }
    }

    /// Renders one output. A frame goes to the subscribers only if something changed.
    pub(crate) fn render(&mut self, id: DeskId, index: usize, read: Read) -> Option<Frame> {
        let desk = self.desks.get_mut(&id)?;
        let out = desk.outputs.get_mut(index)?;
        out.scheduled = false;
        out.last_render = Some(Instant::now());
        self.renders += 1;

        let read = match read {
            Read::OnDamage if out.subscribers.is_empty() => Read::Never,
            other => other,
        };
        let rendered = match crate::render::render(&mut self.gfx, out, &desk.space, read) {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("render: {e}");
                return None;
            }
        };

        let output = out.output.clone();
        let size = output.current_mode().map(|m| m.size).unwrap_or_default();
        let mut frame = None;
        if let Some(pixels) = rendered.pixels {
            let damaged = rendered.damage.is_some();
            if damaged {
                out.serial += 1;
            }
            let f = Frame {
                serial: out.serial,
                time: Instant::now(),
                size: (size.w as u32, size.h as u32),
                damage: rendered.damage.clone().unwrap_or_default(),
                pixels: Arc::new(pixels),
            };
            if damaged {
                out.subscribers.retain(|s| s.send(f.clone()).is_ok());
                self.frames += out.subscribers.len() as u64;
            }
            frame = Some(f);
        }
        if rendered.damage.is_some() {
            self.host_dirty = true;
        }

        let time = self.start.elapsed();
        for window in desk.space.elements() {
            window.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            });
        }
        for layer in layer_map_for_output(&output).layers() {
            layer.send_frame(&output, time, Some(Duration::ZERO), |_, _| {
                Some(output.clone())
            });
        }
        frame
    }

    pub(crate) fn subscribe(&mut self, id: DeskId, index: usize, tx: mpsc::Sender<Frame>) {
        if let Some(out) = self
            .desks
            .get_mut(&id)
            .and_then(|d| d.outputs.get_mut(index))
        {
            out.subscribers.push(tx);
        }
    }

    pub(crate) fn screenshot(&mut self, id: DeskId, index: usize) -> Result<Frame, Error> {
        self.render(id, index, Read::Always)
            .ok_or_else(|| Error("no such output".into()))
    }

    pub(crate) fn output_infos(&self) -> Vec<OutputInfo> {
        let mut infos = Vec::new();
        for (id, desk) in &self.desks {
            for out in &desk.outputs {
                let geo = desk.space.output_geometry(&out.output).unwrap_or_default();
                let map = layer_map_for_output(&out.output);
                let zone = map.non_exclusive_zone();
                infos.push(OutputInfo {
                    name: out.output.name(),
                    desk: *id,
                    kind: out.kind,
                    geometry: Rect {
                        x: geo.loc.x,
                        y: geo.loc.y,
                        w: geo.size.w,
                        h: geo.size.h,
                    },
                    usable: Rect {
                        x: zone.loc.x,
                        y: zone.loc.y,
                        w: zone.size.w,
                        h: zone.size.h,
                    },
                    layers: map
                        .layers()
                        .map(|l| (l.namespace().to_string(), format!("{:?}", l.layer())))
                        .collect(),
                });
            }
        }
        infos
    }

    pub(crate) fn seat_infos(&self) -> Vec<SeatInfo> {
        self.desks
            .iter()
            .map(|(id, d)| SeatInfo {
                name: d.seat_name.clone(),
                desk: *id,
            })
            .collect()
    }

    pub(crate) fn window_infos(&self, id: DeskId) -> Vec<WindowInfo> {
        let Some(desk) = self.desks.get(&id) else {
            return Vec::new();
        };
        desk.space
            .elements()
            .filter(|w| w.alive())
            .map(|w| {
                let geo = desk.space.element_geometry(w).unwrap_or_default();
                let geometry = Rect {
                    x: geo.loc.x,
                    y: geo.loc.y,
                    w: geo.size.w,
                    h: geo.size.h,
                };
                if let Some(x11) = w.x11_surface() {
                    return WindowInfo {
                        title: x11.title(),
                        app_id: x11.class(),
                        x11: true,
                        geometry,
                    };
                }
                let (title, app_id) = w
                    .toplevel()
                    .map(|t| {
                        with_states(t.wl_surface(), |states| {
                            let data = states
                                .data_map
                                .get::<XdgToplevelSurfaceData>()
                                .map(|d| d.lock().unwrap());
                            data.map(|d| {
                                (
                                    d.title.clone().unwrap_or_default(),
                                    d.app_id.clone().unwrap_or_default(),
                                )
                            })
                            .unwrap_or_default()
                        })
                    })
                    .unwrap_or_default();
                WindowInfo {
                    title,
                    app_id,
                    x11: false,
                    geometry,
                }
            })
            .collect()
    }

    pub(crate) fn seat_snapshot(&self, id: DeskId) -> Option<SeatSnapshot> {
        let desk = self.desks.get(&id)?;
        let pointer = desk.seat.get_pointer()?;
        let keyboard = desk.seat.get_keyboard()?;
        let location = pointer.current_location();
        Some(SeatSnapshot {
            pointer: (location.x, location.y),
            pointer_focus: pointer.current_focus().map(|s| format!("{:?}", s.id())),
            keyboard_focus: keyboard
                .current_focus()
                .and_then(|f| f.wl_surface().map(|s| format!("{:?}", s.id()))),
            selection: desk.selection.clone(),
            input_events: desk.input_events,
        })
    }

    pub(crate) fn stats(&self) -> Stats {
        let read = |path: &str| std::fs::read_to_string(path).unwrap_or_default();
        let first = |text: String, n: usize| {
            text.split_whitespace()
                .nth(n)
                .and_then(|v| v.parse::<u64>().ok())
        };
        Stats {
            cpu: Duration::from_nanos(first(read("/proc/thread-self/schedstat"), 0).unwrap_or(0)),
            rss_bytes: first(read("/proc/self/statm"), 1).unwrap_or(0) * 4096,
            renders: self.renders,
            frames: self.frames,
        }
    }

    fn after_dispatch(&mut self) {
        for desk in self.desks.values_mut() {
            desk.space.refresh();
        }
        self.popups.cleanup();
        if self.host_dirty {
            self.host_dirty = false;
            crate::nested::present(self);
        }
        let _ = self.dh.flush_clients();
    }
}

/// Runs a compositor on the calling thread until it is stopped.
///
/// `on_ready` gets the control handle once the person's desk exists.
pub fn run(config: Config, on_ready: impl FnOnce(Handle)) -> Result<(), Error> {
    let err = |e: &dyn std::fmt::Display| Error(e.to_string());
    let mut event_loop: EventLoop<'static, State> = EventLoop::try_new().map_err(|e| err(&e))?;
    let mut display: Display<State> = Display::new().map_err(|e| err(&e))?;
    let dh = display.handle();
    let loop_handle = event_loop.handle();

    let runtime_dir = match config.runtime_dir.clone() {
        Some(dir) => dir,
        None => std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .ok_or_else(|| {
                Error("XDG_RUNTIME_DIR is not set and no runtime_dir was given".into())
            })?,
    };

    let gfx = match config.backend {
        Backend::Headless => Gfx::headless()?,
        Backend::Nested => crate::nested::init(&config, &loop_handle)?,
    };

    let mut state = State {
        start: Instant::now(),
        compositor_state: CompositorState::new::<State>(&dh),
        xdg_shell_state: XdgShellState::new::<State>(&dh),
        shm_state: ShmState::new::<State>(&dh, vec![]),
        seat_state: SeatState::new(),
        data_device_state: DataDeviceState::new::<State>(&dh),
        // Panels and overlays are the shell's business: only clients of the
        // person's desk may create them.
        layer_shell_state: WlrLayerShellState::new_with_filter::<State, _>(&dh, |c| {
            desk_of(c) == Some(PERSON)
        }),
        xwayland_shell_state: XWaylandShellState::new::<State>(&dh),
        popups: PopupManager::default(),
        _output_manager_state: OutputManagerState::new_with_xdg_output::<State>(&dh),
        _viewporter_state: ViewporterState::new::<State>(&dh),
        desks: BTreeMap::new(),
        closing_xwms: Vec::new(),
        next_desk: 0,
        gfx,
        backend: config.backend,
        runtime_dir,
        name: config.name.clone(),
        renders: 0,
        frames: 0,
        host_dirty: true,
        loop_signal: event_loop.get_signal(),
        loop_handle: loop_handle.clone(),
        dh,
    };
    state.gfx.init_protocols(&state.dh);

    let poll_fd = display
        .backend()
        .poll_fd()
        .try_clone_to_owned()
        .map_err(|e| err(&e))?;
    loop_handle
        .insert_source(
            Generic::new(poll_fd, Interest::READ, Mode::Level),
            move |_, _, state| {
                display.dispatch_clients(state)?;
                Ok(PostAction::Continue)
            },
        )
        .map_err(|e| err(&e))?;

    let (tx, rx) = channel::channel::<Job>();
    loop_handle
        .insert_source(rx, |event, _, state| {
            if let ChannelEvent::Msg(job) = event {
                job(state);
            }
        })
        .map_err(|e| err(&e))?;

    let outputs: Vec<_> = config
        .outputs
        .iter()
        .map(|o| (o.name.clone(), o.size))
        .collect();
    let person = state.create_desk(DeskKind::Person, &outputs)?;
    let person = DeskHandle {
        tx: tx.clone(),
        id: person,
        kind: DeskKind::Person,
        socket: state.desks[&person].socket.clone(),
        x11_display: None,
    };
    on_ready(Handle { tx, person });

    event_loop
        .run(None, &mut state, |state| state.after_dispatch())
        .map_err(|e| err(&e))?;

    let ids: Vec<DeskId> = state.desks.keys().copied().collect();
    for id in ids {
        if let Some(desk) = state.desks.get(&id) {
            let _ = std::fs::remove_file(&desk.socket);
            let _ = std::fs::remove_file(desk.socket.with_extension("lock"));
        }
        state.close_desk(id);
    }
    Ok(())
}

impl State {
    /// Where the next window of a desk goes and how large it should be.
    pub(crate) fn placement(
        &mut self,
        id: DeskId,
    ) -> (
        smithay::utils::Point<i32, smithay::utils::Logical>,
        Option<smithay::utils::Size<i32, smithay::utils::Logical>>,
    ) {
        let Some(desk) = self.desks.get_mut(&id) else {
            return ((0, 0).into(), None);
        };
        // New windows open on the output the desk's pointer is on.
        let pointer = desk
            .seat
            .get_pointer()
            .map(|p| p.current_location())
            .unwrap_or_default();
        let under = desk.space.output_under(pointer).next().cloned();
        let Some(output) = under.or_else(|| desk.outputs.first().map(|o| o.output.clone())) else {
            return ((0, 0).into(), None);
        };
        let origin = desk
            .space
            .output_geometry(&output)
            .map(|g| g.loc)
            .unwrap_or_default();
        let mut zone = layer_map_for_output(&output).non_exclusive_zone();
        zone.loc += origin;
        match desk.kind {
            // A space shows one thing at a time: windows fill its output.
            DeskKind::Space => (zone.loc, Some(zone.size)),
            DeskKind::Person => {
                let step = 32 * (desk.cascade % 8);
                desk.cascade += 1;
                (
                    (zone.loc.x + 24 + step, zone.loc.y + 24 + step).into(),
                    None,
                )
            }
        }
    }

    /// Moves windows out from under panels after a panel reserved room.
    pub(crate) fn keep_in_usable_area(&mut self, id: DeskId) {
        let Some(desk) = self.desks.get_mut(&id) else {
            return;
        };
        if desk.kind != DeskKind::Person {
            return;
        }
        let mut moves = Vec::new();
        for out in &desk.outputs {
            let Some(geo) = desk.space.output_geometry(&out.output) else {
                continue;
            };
            let mut zone = layer_map_for_output(&out.output).non_exclusive_zone();
            zone.loc += geo.loc;
            for window in desk.space.elements() {
                let Some(loc) = desk.space.element_location(window) else {
                    continue;
                };
                if !geo.contains(loc) {
                    continue;
                }
                let kept = (loc.x.max(zone.loc.x), loc.y.max(zone.loc.y + 24));
                if kept != (loc.x, loc.y) && loc.y < zone.loc.y {
                    moves.push((window.clone(), kept));
                }
            }
        }
        for (window, loc) in moves {
            desk.space.map_element(window, loc, false);
        }
    }

    /// The keyboard goes to the topmost window that is left.
    pub(crate) fn refocus(&mut self, id: DeskId) {
        let top = self
            .desks
            .get(&id)
            .and_then(|d| d.space.elements().rev().find(|w| w.alive()).cloned());
        self.focus_window(id, top.as_ref());
    }
}
