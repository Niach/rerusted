use std::path::Path;
use std::process::ExitCode;

use vm::{DEFAULT_SIZE, Error, Family, Result, Rig, STAND_IN_SESSION, TARGET_MOUNT};

const USAGE: &str = "\
Tier 3 of the dev rig: host-built binaries in a QEMU/KVM guest with two monitors.

  cargo vm run [options]          build on the host, start or reuse the guest,
                                  restart the session, stream its log
      --bin <name>                launch target/<profile>/<name> as the session
      --session <command>         launch any guest shell command as the session
                                  (default: the stock stand-in, sway)
      --package <name>            build only this package (default: the workspace)
      --release                   build and launch the release profile
      --no-build                  skip the build
      --detach                    return once the session has restarted
  cargo vm image [family|all]     build the base image (default: all three)
  cargo vm unplug <head>          disconnect monitor 0 or 1
  cargo vm plug <head> [WxH]      connect it again (default 1280x800)
  cargo vm screenshot <head> <file.png>
  cargo vm ssh [command]          a shell or one command in the guest
  cargo vm status
  cargo vm stop

Every command takes --family apt|rpm|pacman (default apt, the host's family).
";

fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("vm: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Removes `--name value` from the arguments and returns the value.
fn option(args: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    let Some(at) = args.iter().position(|a| a == name) else {
        return Ok(None);
    };
    if at + 1 >= args.len() {
        return Err(Error::new(format!("{name} needs a value")));
    }
    args.remove(at);
    Ok(Some(args.remove(at)))
}

/// Removes `--name` from the arguments and says whether it was there.
fn flag(args: &mut Vec<String>, name: &str) -> bool {
    let before = args.len();
    args.retain(|a| a != name);
    args.len() != before
}

fn head(args: &[String]) -> Result<u32> {
    args.first()
        .and_then(|h| h.parse().ok())
        .ok_or_else(|| Error::new("expected a head number, 0 or 1"))
}

fn run(mut args: Vec<String>) -> Result<u8> {
    if args.is_empty() || flag(&mut args, "--help") || flag(&mut args, "-h") {
        print!("{USAGE}");
        return Ok(0);
    }
    let command = args.remove(0);
    // Everything after `ssh` belongs to the guest command.
    let family = if command == "ssh" && args.first().is_none_or(|a| a != "--family") {
        Family::Apt
    } else {
        match option(&mut args, "--family")? {
            Some(name) => Family::parse(&name)?,
            None => Family::Apt,
        }
    };
    let rig = Rig::new(family)?;

    match command.as_str() {
        "run" => {
            let release = flag(&mut args, "--release");
            let no_build = flag(&mut args, "--no-build");
            let detach = flag(&mut args, "--detach");
            let package = option(&mut args, "--package")?;
            let bin = option(&mut args, "--bin")?;
            let session = option(&mut args, "--session")?;
            if let Some(extra) = args.first() {
                return Err(Error::new(format!("unknown argument '{extra}'")));
            }
            if !no_build {
                vm::build(package.as_deref(), release)?;
            }
            let target = vm::target_dir()?;
            let session = match (bin, session) {
                (Some(_), Some(_)) => return Err(Error::new("give --bin or --session, not both")),
                (None, Some(session)) => session,
                (None, None) => STAND_IN_SESSION.to_string(),
                (Some(bin), None) => {
                    let profile = if release { "release" } else { "debug" };
                    if !target.join(profile).join(&bin).is_file() {
                        return Err(Error::new(format!(
                            "{} has no binary '{bin}'",
                            target.join(profile).display()
                        )));
                    }
                    format!("{TARGET_MOUNT}/{profile}/{bin}")
                }
            };
            rig.up(&target)?;
            rig.session(&session)?;
            eprintln!("vm: session restarted: {session}");
            if detach {
                return Ok(0);
            }
            Ok(rig.follow_session()? as u8)
        }
        "image" => {
            let families = match args.first().map(String::as_str) {
                None | Some("all") => Family::ALL.to_vec(),
                Some(name) => vec![Family::parse(name)?],
            };
            for family in families {
                let image = Rig::new(family)?.build_image()?;
                println!("{}", image.display());
            }
            Ok(0)
        }
        "unplug" => {
            rig.unplug(head(&args)?)?;
            Ok(0)
        }
        "plug" => {
            let (width, height) = match args.get(1) {
                None => DEFAULT_SIZE,
                Some(size) => size
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .ok_or_else(|| Error::new("expected a size like 1920x1080"))?,
            };
            rig.plug(head(&args)?, width, height)?;
            Ok(0)
        }
        "screenshot" => {
            let file = args
                .get(1)
                .ok_or_else(|| Error::new("expected a file to write"))?;
            rig.screenshot(head(&args)?, Path::new(file))?;
            Ok(0)
        }
        "ssh" => Ok(rig.ssh_attached(&args)? as u8),
        "status" => {
            let state = if rig.is_up() { "running" } else { "stopped" };
            let image = if rig.image().exists() {
                "built"
            } else {
                "not built"
            };
            println!("{} guest: {state}, base image: {image}", family.name());
            Ok(0)
        }
        "stop" => {
            rig.down()?;
            Ok(0)
        }
        other => Err(Error::new(format!(
            "unknown command '{other}', see `cargo vm --help`"
        ))),
    }
}
