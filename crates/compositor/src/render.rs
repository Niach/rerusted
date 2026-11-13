//! Rendering of one output into its own off-screen target.
//!
//! Every output, shown or not, renders into a target of its own. The headless
//! backend uses pixman in memory, the nested one GLES textures that the host
//! window then shows without a copy through the CPU.

use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            Bind, ExportMem, Offscreen,
            element::surface::WaylandSurfaceRenderElement,
            gles::{GlesRenderer, GlesTexture},
            pixman::PixmanRenderer,
        },
    },
    desktop::{Space, Window, space::render_output},
    reexports::{
        pixman::{FormatCode, Image},
        wayland_server::DisplayHandle,
    },
    utils::Rectangle,
};

use crate::{
    api::{Error, Rect},
    nested::Nested,
    state::OutputState,
};

/// The background of an output.
const CLEAR: [f32; 4] = [0.105, 0.086, 0.078, 1.0];

pub(crate) enum Gfx {
    Pixman(Box<PixmanRenderer>),
    Gles(Box<Nested>),
}

pub(crate) enum Target {
    Pixman(Image<'static, 'static>),
    Gles(GlesTexture),
}

/// Whether to read the pixels back after a render.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Read {
    Never,
    OnDamage,
    Always,
}

pub(crate) struct Rendered {
    /// `None` when nothing changed.
    pub damage: Option<Vec<Rect>>,
    pub pixels: Option<Vec<u8>>,
}

impl Gfx {
    pub(crate) fn headless() -> Result<Gfx, Error> {
        PixmanRenderer::new()
            .map(|r| Gfx::Pixman(Box::new(r)))
            .map_err(|e| Error(format!("pixman: {e}")))
    }

    pub(crate) fn init_protocols(&mut self, _dh: &DisplayHandle) {}
}

macro_rules! draw {
    ($renderer:expr, $target:expr, $out:expr, $space:expr, $read:expr, $age:expr, $ty:ty) => {{
        let renderer: &mut $ty = $renderer;
        let size = $out
            .output
            .current_mode()
            .map(|m| m.size)
            .unwrap_or_default();
        let mut fb = renderer
            .bind($target)
            .map_err(|e| Error(format!("bind: {e}")))?;
        let damage = {
            let result = render_output::<_, WaylandSurfaceRenderElement<$ty>, _, _>(
                &$out.output,
                renderer,
                &mut fb,
                1.0,
                $age,
                [$space],
                &[],
                &mut $out.tracker,
                CLEAR,
            )
            .map_err(|e| Error(format!("render: {e:?}")))?;
            result.damage.map(|rects| {
                rects
                    .iter()
                    .map(|r| Rect {
                        x: r.loc.x,
                        y: r.loc.y,
                        w: r.size.w,
                        h: r.size.h,
                    })
                    .collect::<Vec<_>>()
            })
        };
        let want = match $read {
            Read::Never => false,
            Read::OnDamage => damage.is_some(),
            Read::Always => true,
        };
        let pixels = if want {
            let region = Rectangle::from_size((size.w, size.h).into());
            let mapping = renderer
                .copy_framebuffer(&fb, region, Fourcc::Abgr8888)
                .map_err(|e| Error(format!("read back: {e}")))?;
            Some(
                renderer
                    .map_texture(&mapping)
                    .map_err(|e| Error(format!("read back: {e}")))?
                    .to_vec(),
            )
        } else {
            None
        };
        Rendered { damage, pixels }
    }};
}

pub(crate) fn render(
    gfx: &mut Gfx,
    out: &mut OutputState,
    space: &Space<Window>,
    read: Read,
) -> Result<Rendered, Error> {
    let size = out
        .output
        .current_mode()
        .map(|m| m.size)
        .unwrap_or_default();
    // The target keeps the previous frame, so after the first render only the
    // damage is redrawn.
    let age = if out.target.is_some() { 1 } else { 0 };
    match gfx {
        Gfx::Pixman(renderer) => {
            if out.target.is_none() {
                let image =
                    Image::new(FormatCode::A8R8G8B8, size.w as usize, size.h as usize, true)
                        .map_err(|_| Error("pixman: cannot create the output image".into()))?;
                out.target = Some(Target::Pixman(image));
            }
            let Some(Target::Pixman(image)) = out.target.as_mut() else {
                unreachable!()
            };
            Ok(draw!(
                renderer.as_mut(),
                image,
                out,
                space,
                read,
                age,
                PixmanRenderer
            ))
        }
        Gfx::Gles(nested) => {
            let renderer = nested.backend.renderer();
            if out.target.is_none() {
                let texture: GlesTexture = renderer
                    .create_buffer(Fourcc::Abgr8888, (size.w, size.h).into())
                    .map_err(|e| Error(format!("gles: {e}")))?;
                out.target = Some(Target::Gles(texture));
            }
            let Some(Target::Gles(texture)) = out.target.as_mut() else {
                unreachable!()
            };
            Ok(draw!(
                renderer,
                texture,
                out,
                space,
                read,
                age,
                GlesRenderer
            ))
        }
    }
}
