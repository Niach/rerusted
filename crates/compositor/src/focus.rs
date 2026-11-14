//! What a keyboard can be focused on: a Wayland surface or an X11 window.

use std::borrow::Cow;

use smithay::{
    backend::input::KeyState,
    desktop::Window,
    input::{
        Seat,
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{IsAlive, Serial},
    wayland::seat::WaylandFocus,
    xwayland::X11Surface,
};

use crate::state::State;

#[derive(Debug, Clone, PartialEq)]
pub enum Focus {
    Wl(WlSurface),
    X11(X11Surface),
}

impl Focus {
    pub(crate) fn of(window: &Window) -> Option<Focus> {
        if let Some(x11) = window.x11_surface() {
            Some(Focus::X11(x11.clone()))
        } else {
            window.toplevel().map(|t| Focus::Wl(t.wl_surface().clone()))
        }
    }
}

impl IsAlive for Focus {
    fn alive(&self) -> bool {
        match self {
            Focus::Wl(s) => s.alive(),
            Focus::X11(s) => s.alive(),
        }
    }
}

impl WaylandFocus for Focus {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Focus::Wl(s) => Some(Cow::Borrowed(s)),
            Focus::X11(s) => s.wl_surface().map(Cow::Owned),
        }
    }
}

impl KeyboardTarget<State> for Focus {
    fn enter(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        match self {
            Focus::Wl(s) => KeyboardTarget::enter(s, seat, data, keys, serial),
            Focus::X11(s) => KeyboardTarget::enter(s, seat, data, keys, serial),
        }
    }

    fn leave(&self, seat: &Seat<State>, data: &mut State, serial: Serial) {
        match self {
            Focus::Wl(s) => KeyboardTarget::leave(s, seat, data, serial),
            Focus::X11(s) => KeyboardTarget::leave(s, seat, data, serial),
        }
    }

    fn key(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: u32,
    ) {
        match self {
            Focus::Wl(s) => KeyboardTarget::key(s, seat, data, key, state, serial, time),
            Focus::X11(s) => KeyboardTarget::key(s, seat, data, key, state, serial, time),
        }
    }

    fn modifiers(
        &self,
        seat: &Seat<State>,
        data: &mut State,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        match self {
            Focus::Wl(s) => KeyboardTarget::modifiers(s, seat, data, modifiers, serial),
            Focus::X11(s) => KeyboardTarget::modifiers(s, seat, data, modifiers, serial),
        }
    }
}
