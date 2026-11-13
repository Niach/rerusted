//! Input on a desk's seat. The same functions serve injected input (tests,
//! agents) and the host window's input on the nested backend.

use smithay::{
    backend::input::{ButtonState, KeyState},
    desktop::{WindowSurfaceType, layer_map_for_output},
    input::{
        keyboard::FilterResult,
        pointer::{ButtonEvent, MotionEvent},
    },
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, SERIAL_COUNTER},
    wayland::shell::wlr_layer::Layer,
};

use crate::{api::DeskId, focus::Focus, state::State};

/// Linux input codes for the keys and buttons the tests use.
pub mod keys {
    pub const BTN_LEFT: u32 = 0x110;
    pub const BTN_RIGHT: u32 = 0x111;
    pub const KEY_ESC: u32 = 1;
    pub const KEY_BACKSPACE: u32 = 14;
    pub const KEY_TAB: u32 = 15;
    pub const KEY_ENTER: u32 = 28;
    pub const KEY_LEFTCTRL: u32 = 29;
    pub const KEY_LEFTSHIFT: u32 = 42;
    pub const KEY_SPACE: u32 = 57;
}

/// The key and shift state that produce a character on a US layout.
pub(crate) fn stroke(c: char) -> Option<(u32, bool)> {
    const PLAIN: &[(char, u32)] = &[
        ('1', 2),
        ('2', 3),
        ('3', 4),
        ('4', 5),
        ('5', 6),
        ('6', 7),
        ('7', 8),
        ('8', 9),
        ('9', 10),
        ('0', 11),
        ('-', 12),
        ('=', 13),
        ('\t', 15),
        ('q', 16),
        ('w', 17),
        ('e', 18),
        ('r', 19),
        ('t', 20),
        ('y', 21),
        ('u', 22),
        ('i', 23),
        ('o', 24),
        ('p', 25),
        ('[', 26),
        (']', 27),
        ('\n', 28),
        ('a', 30),
        ('s', 31),
        ('d', 32),
        ('f', 33),
        ('g', 34),
        ('h', 35),
        ('j', 36),
        ('k', 37),
        ('l', 38),
        (';', 39),
        ('\'', 40),
        ('`', 41),
        ('\\', 43),
        ('z', 44),
        ('x', 45),
        ('c', 46),
        ('v', 47),
        ('b', 48),
        ('n', 49),
        ('m', 50),
        (',', 51),
        ('.', 52),
        ('/', 53),
        (' ', 57),
    ];
    const SHIFTED: &[(char, char)] = &[
        ('!', '1'),
        ('@', '2'),
        ('#', '3'),
        ('$', '4'),
        ('%', '5'),
        ('^', '6'),
        ('&', '7'),
        ('*', '8'),
        ('(', '9'),
        (')', '0'),
        ('_', '-'),
        ('+', '='),
        ('{', '['),
        ('}', ']'),
        (':', ';'),
        ('"', '\''),
        ('~', '`'),
        ('|', '\\'),
        ('<', ','),
        ('>', '.'),
        ('?', '/'),
    ];
    let plain = |c: char| PLAIN.iter().find(|(p, _)| *p == c).map(|(_, k)| *k);
    if let Some(key) = plain(c) {
        return Some((key, false));
    }
    if c.is_ascii_uppercase() {
        return plain(c.to_ascii_lowercase()).map(|k| (k, true));
    }
    let base = SHIFTED.iter().find(|(s, _)| *s == c)?.1;
    plain(base).map(|k| (k, true))
}

impl State {
    fn time(&self) -> u32 {
        self.start.elapsed().as_millis() as u32
    }

    /// The surface at a point of a desk, with the surface's own position.
    pub(crate) fn surface_under(
        &self,
        desk: DeskId,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        let desk = self.desks.get(&desk)?;
        let output = desk.space.output_under(pos).next();
        let layer_under = |layers: [Layer; 2]| {
            let output = output?;
            let geo = desk.space.output_geometry(output)?;
            let map = layer_map_for_output(output);
            let rel = pos - geo.loc.to_f64();
            layers.iter().find_map(|l| {
                let layer = map.layer_under(*l, rel)?;
                let loc = map.layer_geometry(layer)?.loc;
                layer
                    .surface_under(rel - loc.to_f64(), WindowSurfaceType::ALL)
                    .map(|(s, p)| (s, (p + loc + geo.loc).to_f64()))
            })
        };
        layer_under([Layer::Overlay, Layer::Top])
            .or_else(|| {
                desk.space.element_under(pos).and_then(|(window, loc)| {
                    window
                        .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                        .map(|(s, p)| (s, (p + loc).to_f64()))
                })
            })
            .or_else(|| layer_under([Layer::Bottom, Layer::Background]))
    }

    pub(crate) fn pointer_motion(&mut self, desk: DeskId, pos: Point<f64, Logical>) {
        let Some(d) = self.desks.get_mut(&desk) else {
            return;
        };
        d.input_events += 1;
        let seat = d.seat.clone();
        let under = self.surface_under(desk, pos);
        let Some(pointer) = seat.get_pointer() else {
            return;
        };
        let event = MotionEvent {
            location: pos,
            serial: SERIAL_COUNTER.next_serial(),
            time: self.time(),
        };
        pointer.motion(self, under, &event);
        pointer.frame(self);
    }

    pub(crate) fn pointer_button(&mut self, desk: DeskId, button: u32, pressed: bool) {
        let Some(d) = self.desks.get_mut(&desk) else {
            return;
        };
        d.input_events += 1;
        let seat = d.seat.clone();
        let Some(pointer) = seat.get_pointer() else {
            return;
        };
        let serial = SERIAL_COUNTER.next_serial();
        if pressed && !pointer.is_grabbed() {
            // Click to focus: the window under the pointer gets the keyboard.
            let window = d
                .space
                .element_under(pointer.current_location())
                .map(|(w, _)| w.clone());
            if let Some(window) = window {
                d.space.raise_element(&window, true);
                self.focus_window(desk, Some(&window));
            }
        }
        let event = ButtonEvent {
            button,
            state: if pressed {
                ButtonState::Pressed
            } else {
                ButtonState::Released
            },
            serial,
            time: self.time(),
        };
        pointer.button(self, &event);
        pointer.frame(self);
    }

    pub(crate) fn key(&mut self, desk: DeskId, key: u32, pressed: bool) {
        let Some(d) = self.desks.get_mut(&desk) else {
            return;
        };
        d.input_events += 1;
        let seat = d.seat.clone();
        let Some(keyboard) = seat.get_keyboard() else {
            return;
        };
        let state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };
        let time = self.time();
        // xkb key codes are Linux key codes plus 8.
        keyboard.input::<(), _>(
            self,
            (key + 8).into(),
            state,
            SERIAL_COUNTER.next_serial(),
            time,
            |_, _, _| FilterResult::Forward,
        );
    }

    /// Gives the desk's keyboard to a window, or to nothing.
    pub(crate) fn focus_window(&mut self, desk: DeskId, window: Option<&smithay::desktop::Window>) {
        let Some(d) = self.desks.get_mut(&desk) else {
            return;
        };
        let seat = d.seat.clone();
        for w in d.space.elements() {
            let active = Some(w) == window;
            if w.set_activated(active)
                && let Some(t) = w.toplevel().filter(|t| t.is_initial_configure_sent())
            {
                t.send_pending_configure();
            }
        }
        if let Some(keyboard) = seat.get_keyboard() {
            keyboard.set_focus(
                self,
                window.and_then(Focus::of),
                SERIAL_COUNTER.next_serial(),
            );
        }
    }
}
