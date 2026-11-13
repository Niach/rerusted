//! Three gpui surfaces as ordinary Wayland clients: a window, a panel and an
//! overlay. The panel and the overlay use the layer-shell protocol.
//!
//! The colours are the rusted theme tokens from `design/generated/tokens.hex.json`.

use gpui::{
    App, Bounds, Context, MouseButton, Size, Window, WindowBackgroundAppearance, WindowBounds,
    WindowKind, WindowOptions, div, layer_shell::*, point, prelude::*, px, rgb,
};
use gpui_platform::application;

const BACKGROUND: u32 = 0x0f0807;
const CARD: u32 = 0x1e1512;
const FOREGROUND: u32 = 0xfcf9f7;
const ACCENT: u32 = 0xba4f1f;
const BORDER: u32 = 0x61463e;

/// A window whose big area changes colour on every click.
struct Demo {
    clicks: u32,
}

impl Render for Demo {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fill = if self.clicks % 2 == 0 { CARD } else { ACCENT };
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(fill))
            .text_color(rgb(FOREGROUND))
            .child(format!("gpui window, clicked {} times", self.clicks))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.clicks += 1;
                    cx.notify();
                }),
            )
    }
}

struct Panel;

impl Render for Panel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .px_3()
            .gap_3()
            .bg(rgb(BACKGROUND))
            .border_b_1()
            .border_color(rgb(BORDER))
            .text_color(rgb(FOREGROUND))
            .child(div().size_4().rounded_sm().bg(rgb(ACCENT)))
            .child("gpui panel (layer shell, top)")
    }
}

struct Overlay;

impl Render for Overlay {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(ACCENT))
            .rounded_lg()
            .text_color(rgb(FOREGROUND))
            .child("gpui overlay")
    }
}

fn layer(namespace: &str, size: (f32, f32), options: LayerShellOptions) -> WindowOptions {
    WindowOptions {
        titlebar: None,
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(0.), px(0.)),
            size: Size::new(px(size.0), px(size.1)),
        })),
        app_id: Some(namespace.to_string()),
        window_background: WindowBackgroundAppearance::Transparent,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: namespace.to_string(),
            ..options
        }),
        ..Default::default()
    }
}

fn main() {
    application().run(|cx: &mut App| {
        let window = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: point(px(0.), px(0.)),
                size: Size::new(px(480.), px(320.)),
            })),
            app_id: Some("gpui-window".to_string()),
            ..Default::default()
        };
        cx.open_window(window, |_, cx| cx.new(|_| Demo { clicks: 0 }))
            .expect("a gpui window");

        // The answer the spike is after: does gpui open layer-shell surfaces as it is?
        let panel = layer(
            "panel",
            (960., 32.),
            LayerShellOptions {
                layer: Layer::Top,
                anchor: Anchor::TOP | Anchor::LEFT | Anchor::RIGHT,
                exclusive_zone: Some(px(32.)),
                keyboard_interactivity: KeyboardInteractivity::None,
                ..Default::default()
            },
        );
        match cx.open_window(panel, |_, cx| cx.new(|_| Panel)) {
            Ok(_) => println!("panel: ok"),
            Err(e) => println!("panel: failed: {e:#}"),
        }

        let overlay = layer(
            "overlay",
            (240., 64.),
            LayerShellOptions {
                layer: Layer::Overlay,
                anchor: Anchor::BOTTOM | Anchor::RIGHT,
                margin: Some((px(0.), px(24.), px(24.), px(0.))),
                keyboard_interactivity: KeyboardInteractivity::None,
                ..Default::default()
            },
        );
        match cx.open_window(overlay, |_, cx| cx.new(|_| Overlay)) {
            Ok(_) => println!("overlay: ok"),
            Err(e) => println!("overlay: failed: {e:#}"),
        }
    });
}
