//! The nested backend: one window on the host desktop that shows the person's
//! outputs side by side and every agent space as a read-only tile next to them.

use smithay::{
    backend::{
        input::{
            AbsolutePositionEvent, ButtonState, InputEvent, KeyState, KeyboardKeyEvent,
            PointerButtonEvent,
        },
        renderer::{
            Renderer,
            damage::OutputDamageTracker,
            element::{Id, Kind, texture::TextureRenderElement},
            gles::{GlesRenderer, GlesTexture},
        },
        winit::{self, WinitEvent, WinitGraphicsBackend},
    },
    reexports::{
        calloop::LoopHandle,
        winit::{dpi::PhysicalSize, window::Window},
    },
    utils::{Physical, Point, Size, Transform},
};

use crate::{
    api::{Config, DeskKind, Error},
    render::{Gfx, Target},
    state::{PERSON, State},
};

/// Width of the column that shows the spaces.
const TILE: i32 = 400;

pub(crate) struct Nested {
    pub backend: WinitGraphicsBackend<GlesRenderer>,
    tracker: Option<(Size<i32, Physical>, OutputDamageTracker)>,
}

pub(crate) fn init(
    config: &Config,
    loop_handle: &LoopHandle<'static, State>,
) -> Result<Gfx, Error> {
    let width: i32 = config.outputs.iter().map(|o| o.size.0).sum::<i32>() + TILE;
    let height: i32 = config.outputs.iter().map(|o| o.size.1).max().unwrap_or(600);
    let attributes = Window::default_attributes()
        .with_title("rerusted compositor (nested)")
        .with_inner_size(PhysicalSize::new(width as u32, height as u32));
    let (backend, events) = winit::init_from_attributes::<GlesRenderer>(attributes)
        .map_err(|e| Error(format!("winit: {e}")))?;

    loop_handle
        .insert_source(events, |event, _, state| match event {
            WinitEvent::Input(event) => host_input(state, event),
            WinitEvent::Redraw | WinitEvent::Resized { .. } => state.host_dirty = true,
            WinitEvent::CloseRequested => state.loop_signal.stop(),
            WinitEvent::Focus(_) => {}
        })
        .map_err(|e| Error(e.to_string()))?;

    Ok(Gfx::Gles(Box::new(Nested {
        backend,
        tracker: None,
    })))
}

/// Input in the host window drives the person's seat, and only that one.
fn host_input(state: &mut State, event: InputEvent<winit::WinitInput>) {
    match event {
        InputEvent::Keyboard { event } => {
            let key = event.key_code().raw().saturating_sub(8);
            state.key(PERSON, key, event.state() == KeyState::Pressed);
        }
        InputEvent::PointerMotionAbsolute { event } => {
            let Gfx::Gles(nested) = &state.gfx else {
                return;
            };
            let size = nested.backend.window_size().to_logical(1);
            let pos = event.position_transformed(size);
            let inside = state
                .desks
                .get(&PERSON)
                .is_some_and(|d| d.space.output_under(pos).next().is_some());
            if inside {
                state.pointer_motion(PERSON, pos);
            }
        }
        InputEvent::PointerButton { event } => {
            state.pointer_button(
                PERSON,
                event.button_code(),
                event.state() == ButtonState::Pressed,
            );
        }
        _ => {}
    }
}

/// Draws every output's texture into the host window.
pub(crate) fn present(state: &mut State) {
    let State { gfx, desks, .. } = state;
    let Gfx::Gles(nested) = gfx else { return };
    let Nested { backend, tracker } = nested.as_mut();
    let window = backend.window_size();
    if tracker.as_ref().is_none_or(|(size, _)| *size != window) {
        *tracker = Some((
            window,
            OutputDamageTracker::new(window, 1.0, Transform::Flipped180),
        ));
    }
    let Some((_, tracker)) = tracker.as_mut() else {
        return;
    };

    let context = backend.renderer().context_id();
    let mut elements: Vec<TextureRenderElement<GlesTexture>> = Vec::new();
    let mut tile_y = 0;
    let person_width: i32 = desks
        .get(&PERSON)
        .map(|d| {
            d.outputs
                .iter()
                .filter_map(|o| o.output.current_mode())
                .map(|m| m.size.w)
                .sum()
        })
        .unwrap_or(0);
    for desk in desks.values() {
        for out in &desk.outputs {
            let Some(Target::Gles(texture)) = &out.target else {
                continue;
            };
            let Some(mode) = out.output.current_mode() else {
                continue;
            };
            let (location, size): (Point<i32, Physical>, Size<i32, _>) = match desk.kind {
                DeskKind::Person => {
                    let loc = desk
                        .space
                        .output_geometry(&out.output)
                        .map(|g| g.loc)
                        .unwrap_or_default();
                    ((loc.x, loc.y).into(), (mode.size.w, mode.size.h).into())
                }
                DeskKind::Space => {
                    let w = TILE - 16;
                    let h = mode.size.h * w / mode.size.w.max(1);
                    let loc = (person_width + 8, tile_y + 8);
                    tile_y += h + 8;
                    (loc.into(), (w, h).into())
                }
            };
            elements.push(TextureRenderElement::from_static_texture(
                Id::new(),
                context.clone(),
                location.to_f64(),
                texture.clone(),
                1,
                Transform::Normal,
                None,
                None,
                Some(size),
                None,
                Kind::Unspecified,
            ));
        }
    }

    {
        let Ok((renderer, mut fb)) = backend.bind() else {
            return;
        };
        if let Err(e) =
            tracker.render_output(renderer, &mut fb, 0, &elements, [0.04, 0.04, 0.045, 1.0])
        {
            tracing::error!("host window: {e:?}");
        }
    }
    if let Err(e) = backend.submit(None) {
        tracing::error!("host window: {e}");
    }
}
