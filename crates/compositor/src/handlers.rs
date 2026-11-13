//! The protocol handlers. Each one first finds the desk the client belongs to.

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    delegate_compositor, delegate_data_device, delegate_layer_shell, delegate_output,
    delegate_seat, delegate_shm, delegate_viewporter, delegate_xdg_shell, delegate_xwayland_shell,
    desktop::{LayerSurface, PopupKind, Window, WindowSurfaceType, layer_map_for_output},
    input::{Seat, SeatHandler, SeatState, pointer::CursorImageStatus},
    output::Output,
    reexports::wayland_server::{
        Client, Resource,
        protocol::{
            wl_buffer::WlBuffer, wl_output::WlOutput, wl_seat::WlSeat, wl_surface::WlSurface,
        },
    },
    utils::{Logical, Rectangle, Serial},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            CompositorClientState, CompositorHandler, CompositorState, get_parent,
            is_sync_subsurface, with_states,
        },
        output::OutputHandler,
        seat::WaylandFocus,
        selection::{
            SelectionHandler, SelectionSource, SelectionTarget,
            data_device::{
                ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
                set_data_device_focus,
            },
        },
        shell::{
            wlr_layer::{
                Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler,
                WlrLayerShellState,
            },
            xdg::{PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState},
        },
        shm::{ShmHandler, ShmState},
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        X11Surface, X11Wm, XWaylandClientData, XwmHandler,
        xwm::{Reorder, ResizeEdge, XwmId},
    },
};

use crate::{
    api::DeskId,
    focus::Focus,
    state::{ClientState, State},
};

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        if let Some(state) = client.get_data::<XWaylandClientData>() {
            return &state.compositor_state;
        }
        &client
            .get_data::<ClientState>()
            .expect("every client carries ClientState")
            .compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        let Some(id) = self.desk_of_surface(surface) else {
            return;
        };
        let Some(desk) = self.desks.get(&id) else {
            return;
        };

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = desk
                .space
                .elements()
                .find(|w| w.wl_surface().as_deref() == Some(&root))
            {
                window.on_commit();
            }
        }

        let toplevel = desk
            .space
            .elements()
            .filter_map(|w| w.toplevel())
            .find(|t| t.wl_surface() == surface);
        if let Some(toplevel) = toplevel
            && !toplevel.is_initial_configure_sent()
        {
            toplevel.send_configure();
        }

        self.popups.commit(surface);
        if let Some(PopupKind::Xdg(popup)) = self.popups.find_popup(surface)
            && !popup.is_initial_configure_sent()
        {
            let _ = popup.send_configure();
        }

        let mut arranged = false;
        for out in &desk.outputs {
            let mut map = layer_map_for_output(&out.output);
            let Some(layer) = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .cloned()
            else {
                continue;
            };
            map.arrange();
            let configured = with_states(surface, |states| {
                states
                    .data_map
                    .get::<LayerSurfaceData>()
                    .map(|d| d.lock().unwrap().initial_configure_sent)
                    .unwrap_or(false)
            });
            if !configured {
                layer.layer_surface().send_configure();
            }
            arranged = true;
        }
        if arranged {
            self.keep_in_usable_area(id);
        }

        self.schedule(id);
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let Some(id) = self.desk_of_surface(surface.wl_surface()) else {
            return;
        };
        let (location, size) = self.placement(id);
        if let Some(size) = size {
            surface.with_pending_state(|state| state.size = Some(size));
        }
        let window = Window::new_wayland_window(surface);
        if let Some(desk) = self.desks.get_mut(&id) {
            desk.space.map_element(window.clone(), location, true);
        }
        self.focus_window(id, Some(&window));
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        let found = self.desks.iter().find_map(|(id, desk)| {
            let window = desk
                .space
                .elements()
                .find(|w| w.toplevel() == Some(&surface))?;
            Some((*id, window.clone()))
        });
        if let Some((id, window)) = found {
            if let Some(desk) = self.desks.get_mut(&id) {
                desk.space.unmap_elem(&window);
            }
            self.refocus(id);
            self.schedule(id);
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        surface: PopupSurface,
        positioner: PositionerState,
        token: u32,
    ) {
        surface.with_pending_state(|state| {
            state.geometry = positioner.get_geometry();
            state.positioner = positioner;
        });
        surface.send_repositioned(token);
    }
}

impl SeatHandler for State {
    type KeyboardFocus = Focus;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<State> {
        &mut self.seat_state
    }

    fn cursor_image(&mut self, _seat: &Seat<Self>, _image: CursorImageStatus) {}

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&Focus>) {
        // The clipboard follows the keyboard of this seat only.
        let client = focused
            .and_then(|f| f.wl_surface())
            .and_then(|s| self.dh.get_client(s.id()).ok());
        set_data_device_focus(&self.dh, seat, client);
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();

    fn new_selection(
        &mut self,
        ty: SelectionTarget,
        source: Option<SelectionSource>,
        seat: Seat<Self>,
    ) {
        if ty != SelectionTarget::Clipboard {
            return;
        }
        if let Some(desk) = self.desks.values_mut().find(|d| d.seat == seat) {
            desk.selection = source.map(|s| s.mime_types());
        }
    }
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}
impl OutputHandler for State {}

impl WlrLayerShellHandler for State {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        output: Option<WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let Some(id) = self.desk_of_surface(surface.wl_surface()) else {
            return;
        };
        let Some(desk) = self.desks.get(&id) else {
            return;
        };
        let output = output
            .as_ref()
            .and_then(Output::from_resource)
            .filter(|o| desk.outputs.iter().any(|s| &s.output == o))
            .or_else(|| desk.outputs.first().map(|s| s.output.clone()));
        let Some(output) = output else { return };
        let _ = layer_map_for_output(&output).map_layer(&LayerSurface::new(surface, namespace));
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        let mut touched = None;
        for (id, desk) in &self.desks {
            for out in &desk.outputs {
                let mut map = layer_map_for_output(&out.output);
                let layer = map
                    .layers()
                    .find(|l| l.layer_surface() == &surface)
                    .cloned();
                if let Some(layer) = layer {
                    map.unmap_layer(&layer);
                    touched = Some(*id);
                }
            }
        }
        if let Some(id) = touched {
            self.schedule(id);
        }
    }
}

impl XWaylandShellHandler for State {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }

    fn surface_associated(&mut self, xwm: XwmId, _surface: WlSurface, _window: X11Surface) {
        // The surface may have committed its content before it was tied to
        // its X11 window; show it now.
        if let Some(id) = self.desk_of_xwm(xwm) {
            self.schedule(id);
        }
    }
}

impl State {
    fn desk_of_xwm(&self, xwm: XwmId) -> Option<DeskId> {
        self.desks
            .iter()
            .find(|(_, d)| d.xwm.as_ref().is_some_and(|w| w.id() == xwm))
            .map(|(id, _)| *id)
    }

    fn x11_window(&self, id: DeskId, surface: &X11Surface) -> Option<Window> {
        self.desks
            .get(&id)?
            .space
            .elements()
            .find(|w| w.x11_surface() == Some(surface))
            .cloned()
    }
}

impl XwmHandler for State {
    fn xwm_state(&mut self, xwm: XwmId) -> &mut X11Wm {
        self.desks
            .values_mut()
            .filter_map(|d| d.xwm.as_mut())
            .chain(self.closing_xwms.iter_mut())
            .find(|w| w.id() == xwm)
            .expect("an X11 event names a window manager that is still held")
    }

    fn disconnected(&mut self, xwm: XwmId) {
        self.closing_xwms.retain(|w| w.id() != xwm);
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, xwm: XwmId, surface: X11Surface) {
        let Some(id) = self.desk_of_xwm(xwm) else {
            return;
        };
        let _ = surface.set_mapped(true);
        let (location, size) = self.placement(id);
        let size = size.unwrap_or(surface.geometry().size);
        let _ = surface.configure(Some(Rectangle::new(location, size)));
        let window = Window::new_x11_window(surface);
        if let Some(desk) = self.desks.get_mut(&id) {
            desk.space.map_element(window.clone(), location, true);
        }
        self.focus_window(id, Some(&window));
        self.schedule(id);
    }

    fn mapped_override_redirect_window(&mut self, xwm: XwmId, surface: X11Surface) {
        let Some(id) = self.desk_of_xwm(xwm) else {
            return;
        };
        let location = surface.geometry().loc;
        if let Some(desk) = self.desks.get_mut(&id) {
            desk.space
                .map_element(Window::new_x11_window(surface), location, true);
        }
        self.schedule(id);
    }

    fn unmapped_window(&mut self, xwm: XwmId, surface: X11Surface) {
        let Some(id) = self.desk_of_xwm(xwm) else {
            return;
        };
        if let Some(window) = self.x11_window(id, &surface)
            && let Some(desk) = self.desks.get_mut(&id)
        {
            desk.space.unmap_elem(&window);
        }
        if !surface.is_override_redirect() {
            let _ = surface.set_mapped(false);
        }
        self.refocus(id);
        self.schedule(id);
    }

    fn destroyed_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        // A window may ask for a size until it is mapped; after that the desk decides.
        if window.is_mapped() && !window.is_override_redirect() {
            let _ = window.configure(None);
            return;
        }
        let mut geometry = window.geometry();
        if let Some(w) = w {
            geometry.size.w = w as i32;
        }
        if let Some(h) = h {
            geometry.size.h = h as i32;
        }
        let _ = window.configure(geometry);
    }

    fn configure_notify(
        &mut self,
        xwm: XwmId,
        surface: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        if !surface.is_override_redirect() {
            return;
        }
        let Some(id) = self.desk_of_xwm(xwm) else {
            return;
        };
        if let Some(window) = self.x11_window(id, &surface)
            && let Some(desk) = self.desks.get_mut(&id)
        {
            desk.space.map_element(window, geometry.loc, false);
        }
    }

    fn resize_request(
        &mut self,
        _xwm: XwmId,
        _window: X11Surface,
        _button: u32,
        _edge: ResizeEdge,
    ) {
    }
    fn move_request(&mut self, _xwm: XwmId, _window: X11Surface, _button: u32) {}
}

delegate_compositor!(State);
delegate_shm!(State);
delegate_xdg_shell!(State);
delegate_seat!(State);
delegate_data_device!(State);
delegate_output!(State);
delegate_layer_shell!(State);
delegate_xwayland_shell!(State);
delegate_viewporter!(State);
