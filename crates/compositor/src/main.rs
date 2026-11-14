//! `compositor nested` shows the outputs in a window on the host desktop,
//! `compositor headless` runs the same thing with no screen.

#[cfg(target_os = "linux")]
fn main() -> std::process::ExitCode {
    use compositor::{Backend, Config, OutputConfig, SpaceConfig};

    const USAGE: &str = "usage: compositor <nested|headless> [--outputs N] [--spaces N] \
                         [--spawn CMD] [--spawn-in-space CMD] [--no-xwayland]";

    let mut args = std::env::args().skip(1);
    let backend = match args.next().as_deref() {
        Some("nested") => Backend::Nested,
        Some("headless") => Backend::Headless,
        _ => {
            eprintln!("{USAGE}");
            return std::process::ExitCode::from(2);
        }
    };
    let (mut outputs, mut spaces, mut xwayland) = (2usize, 1usize, true);
    let (mut spawn, mut spawn_in_space) = (Vec::new(), Vec::new());
    while let Some(arg) = args.next() {
        let mut value = || args.next().unwrap_or_default();
        match arg.as_str() {
            "--outputs" => outputs = value().parse().unwrap_or(2).max(1),
            "--spaces" => spaces = value().parse().unwrap_or(1),
            "--spawn" => spawn.push(value()),
            "--spawn-in-space" => spawn_in_space.push(value()),
            "--no-xwayland" => xwayland = false,
            _ => {
                eprintln!("{USAGE}");
                return std::process::ExitCode::from(2);
            }
        }
    }

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .init();

    let mut config = Config::new(backend);
    config.outputs = (1..=outputs)
        .map(|n| OutputConfig {
            name: format!("out-{n}"),
            size: (960, 600),
        })
        .collect();

    let result = compositor::run(config, move |handle| {
        // Spaces wait for their XWayland, so they are set up off the loop's thread.
        std::thread::spawn(move || {
            let shell = |desk: &compositor::DeskHandle, line: &str| {
                let mut command = desk.command("sh");
                if let Err(e) = command.arg("-c").arg(line).spawn() {
                    eprintln!("{line}: {e}");
                }
            };
            println!(
                "person: WAYLAND_DISPLAY={}",
                handle.person().socket().display()
            );
            for line in &spawn {
                shell(handle.person(), line);
            }
            for n in 0..spaces {
                match handle.create_space(SpaceConfig {
                    xwayland,
                    ..SpaceConfig::default()
                }) {
                    Ok(space) => {
                        let display = space
                            .x11_display()
                            .map(|d| format!(" DISPLAY=:{d}"))
                            .unwrap_or_default();
                        println!(
                            "space {}: WAYLAND_DISPLAY={}{display}",
                            n + 1,
                            space.socket().display()
                        );
                        for line in &spawn_in_space {
                            shell(&space, line);
                        }
                    }
                    Err(e) => eprintln!("space {}: {e}", n + 1),
                }
            }
        });
    });
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("compositor: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("the compositor runs on Linux only");
}
