# compositor

Spike for #RUST-23: a minimal Wayland compositor on Smithay that has outputs,
seats and agent spaces in its model from the start. Not the product's
compositor yet: no DRM, no real monitors, no layout, no decorations.

## The model

A **desk** is one seat plus the outputs it works on, with its own window
stack, its own Wayland socket and optionally its own XWayland.

- The **person's desk** has the real outputs (here: nested or headless ones).
- An **agent space** is a desk with exactly one virtual output that is never
  scanned out. Its seat is its own, so driving it never touches the person's
  pointer, keyboard focus or clipboard.

A client belongs to the desk whose socket it connected to and sees only that
desk's seat and outputs. Panels and overlays (layer shell) are offered on the
person's desk only. An output is rendered when a client committed something,
never on a timer, and a frame is delivered only if the render had damage.

## Commands

These are tiers 1 and 2 of the dev rig (#RUST-21).

```sh
# Tier 1, nested: the person's two outputs side by side in one window on the
# host desktop, each agent space as a read-only tile to the right.
cargo run -p compositor -- nested

# Tier 2, headless: the same binary with no screen and no GPU.
cargo run -p compositor -- headless

# Tier 2, the tests: every test starts its own headless compositor.
cargo test -p compositor
```

Flags of the binary:

| Flag | Meaning | Default |
|---|---|---|
| `--outputs N` | outputs of the person's desk, 960x600 each | 2 |
| `--spaces N` | agent spaces, 800x600 each | 1 |
| `--spawn CMD` | run a shell command on the person's desk (repeatable) | |
| `--spawn-in-space CMD` | run a shell command in every space (repeatable) | |
| `--no-xwayland` | spaces without an X server | |

The binary prints the `WAYLAND_DISPLAY` (and `DISPLAY`) of each desk, so any
program can be started on one by hand. Example:

```sh
cargo run -p compositor -- nested --spawn "zenity --info --text hello" --spawn-in-space xterm
```

In the nested window, mouse and keyboard drive the person's seat. A new window
opens on the output the pointer is on.

## What it needs

To build: `libwayland-dev libxkbcommon-dev libpixman-1-dev libegl1-mesa-dev`
(Debian names). To run the tests: `xwayland xterm zenity dbus-daemon`.

The crate is Linux only; elsewhere it compiles to nothing.

## Driving a space from code

```rust
let compositor = compositor::start(Config::new(Backend::Headless))?;
let space = compositor.create_space(SpaceConfig::default())?;
let frames = space.frames(0)?;                 // only when something changed
space.command("xterm").spawn()?;               // opens in the space
space.click(200.0, 150.0)?;                    // the space's own pointer
space.type_text("echo hello\n")?;              // the space's own keyboard
let frame = frames.recv()?;                    // pixels plus the damage
```

`tests/space.rs` is the full example: it opens a Wayland app, an X11 app and a
GTK app in a space, clicks, types and reads the result back, and proves the
person's seat saw none of it.

## gpui as a client

`spikes/gpui-shell` is a cargo root of its own (it builds zed's gpui from git)
that opens a gpui window, a gpui panel and a gpui overlay. The test for it is
ignored by default:

```sh
cargo build --manifest-path spikes/gpui-shell/Cargo.toml
GPUI_SHELL=spikes/gpui-shell/target/debug/gpui-shell \
    cargo test -p compositor --test gpui -- --ignored
```

The headless backend takes shared-memory buffers only, so on a machine with a
GPU point gpui at software Vulkan for this test:
`VK_DRIVER_FILES=/usr/share/vulkan/icd.d/lvp_icd.json` (package
`mesa-vulkan-drivers`).

## Measuring

```sh
cargo run -p compositor --release --example measure
```

prints the time from an injected action to the frame that shows it, and the
cost of 1, 4 and 16 idle and busy spaces.

## Known limits

- The headless backend has no dmabuf: clients must use shared memory.
- The nested backend offers no dmabuf either yet, so GPU clients fall back to
  software or fail there (#RUST-23 follow-up).
- No clipboard bridge between a space's X11 apps and its Wayland apps.
- A frame is a full copy of the output. Damage rectangles are reported, but
  the pixels are not yet cut to them.
- Killing the binary with a signal leaves its socket files behind.
