//! The rig itself: builds base images, starts or reuses a guest, restarts
//! the session in it and plugs its monitors.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::net::TcpListener;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::error::{Error, Result};
use crate::qemu::{GPU_ID, Machine, TARGET_TAG};
use crate::qmp::Qmp;
use crate::recipe::{Family, PROVISIONED_MARK, SESSION_UNIT, TARGET_MOUNT, USER, recipe};

/// Number of virtual monitors on the guest's one display device.
pub const HEADS: u32 = 2;
/// Size a head gets when nothing else is asked for.
pub const DEFAULT_SIZE: (u32, u32) = (1280, 800);

/// Journal matches for the session: what the session's processes print
/// (logind moves them out of the unit into a session scope, so they are
/// found by identifier) and what systemd says about the unit itself.
const SESSION_LOG: &str =
    "SYSLOG_IDENTIFIER=rig-session + UNIT=rig-session.service + _SYSTEMD_UNIT=rig-session.service";

const MEMORY_MB: u32 = 4096;
const CPUS: u32 = 4;
const DISK_SIZE: &str = "12G";

pub struct Rig {
    family: Family,
    root: PathBuf,
}

impl Rig {
    /// A rig whose images and state live in `$RERUSTED_VM_DIR`, or in
    /// `rerusted/vm` under the user's cache directory. The directory is
    /// shared by every checkout and worktree.
    pub fn new(family: Family) -> Result<Self> {
        let root = if let Some(dir) = std::env::var_os("RERUSTED_VM_DIR") {
            PathBuf::from(dir)
        } else if let Some(dir) = std::env::var_os("XDG_CACHE_HOME") {
            PathBuf::from(dir).join("rerusted/vm")
        } else if let Some(home) = std::env::var_os("HOME") {
            PathBuf::from(home).join(".cache/rerusted/vm")
        } else {
            return Err(Error::new("neither RERUSTED_VM_DIR nor HOME is set"));
        };
        Ok(Self::at(family, root))
    }

    pub fn at(family: Family, root: PathBuf) -> Self {
        Self { family, root }
    }

    pub fn family(&self) -> Family {
        self.family
    }

    fn run_dir(&self) -> PathBuf {
        self.root.join("run").join(self.family.name())
    }

    fn key(&self) -> PathBuf {
        self.root.join("key")
    }

    /// The base image of this family; its name carries the recipe's fingerprint.
    pub fn image(&self) -> PathBuf {
        let name = format!(
            "{}-{}.qcow2",
            self.family.name(),
            recipe(self.family).fingerprint()
        );
        self.root.join("images").join(name)
    }

    fn bus_address(&self) -> String {
        format!("unix:path={}", self.run_dir().join("bus").display())
    }

    fn monitor(&self) -> Result<Qmp> {
        Qmp::connect(&self.run_dir().join("qmp.sock"))
    }

    /// True when the guest's monitor answers.
    pub fn is_up(&self) -> bool {
        self.monitor().is_ok()
    }

    // ---- base image ----

    /// Builds the base image unless it exists: pinned cloud image, verified,
    /// provisioned once by cloud-init, then flattened into one file.
    pub fn build_image(&self) -> Result<PathBuf> {
        let image = self.image();
        if image.exists() {
            return Ok(image);
        }
        require_kvm()?;
        let recipe = recipe(self.family);
        let cloud_image = self.download()?;
        self.ensure_key()?;

        let build = self.root.join("build").join(self.family.name());
        let _ = fs::remove_dir_all(&build);
        fs::create_dir_all(build.join("seed"))?;
        let public_key = fs::read_to_string(self.key().with_extension("pub"))?;
        fs::write(build.join("seed/user-data"), recipe.user_data(&public_key))?;
        fs::write(build.join("seed/meta-data"), recipe.meta_data())?;
        let disk = build.join("disk.qcow2");
        overlay(&cloud_image, &disk)?;

        let serial_log = build.join("serial.log");
        eprintln!(
            "vm: provisioning {} ({}), log: {}",
            self.family.name(),
            recipe.distro,
            serial_log.display()
        );
        let machine = Machine {
            disk: disk.clone(),
            seed: Some(build.join("seed")),
            ssh_port: free_port()?,
            heads: HEADS,
            memory_mb: MEMORY_MB,
            cpus: CPUS,
            qmp_socket: build.join("qmp.sock"),
            serial_log: serial_log.clone(),
            virtiofs_socket: None,
            display_bus: None,
            pid_file: None,
        };
        let mut qemu = Command::new("qemu-system-x86_64")
            .args(machine.arguments())
            .stdin(Stdio::null())
            .spawn()
            .map_err(|e| Error::new(format!("qemu-system-x86_64: {e}")))?;
        let deadline = Instant::now() + Duration::from_secs(30 * 60);
        while qemu.try_wait()?.is_none() {
            if Instant::now() > deadline {
                let _ = qemu.kill();
                return Err(Error::new(format!(
                    "provisioning did not finish in 30 minutes, see {}",
                    serial_log.display()
                )));
            }
            sleep(Duration::from_secs(1));
        }
        let log = String::from_utf8_lossy(&fs::read(&serial_log)?).into_owned();
        if !log.contains(PROVISIONED_MARK) {
            return Err(Error::new(format!(
                "provisioning failed, see {}",
                serial_log.display()
            )));
        }

        fs::create_dir_all(self.root.join("images"))?;
        let prefix = format!("{}-", self.family.name());
        for entry in fs::read_dir(self.root.join("images"))? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                fs::remove_file(entry.path())?;
            }
        }
        let partial = image.with_extension("partial");
        run(Command::new("qemu-img")
            .args(["convert", "-O", "qcow2"])
            .arg(&disk)
            .arg(&partial))?;
        fs::rename(&partial, &image)?;
        fs::remove_dir_all(&build)?;
        Ok(image)
    }

    fn download(&self) -> Result<PathBuf> {
        let recipe = recipe(self.family);
        let dir = self.root.join("download");
        fs::create_dir_all(&dir)?;
        let file = dir.join(format!(
            "{}-{}.img",
            self.family.name(),
            &recipe.image_sha256[..16]
        ));
        if file.exists() {
            return Ok(file);
        }
        eprintln!("vm: downloading {}", recipe.image_url);
        let partial = file.with_extension("partial");
        run(Command::new("curl")
            .args(["-fsSL", "--retry", "3", "-o"])
            .arg(&partial)
            .arg(recipe.image_url))?;
        let sum = run(Command::new("sha256sum").arg(&partial))?;
        if sum.split_whitespace().next() != Some(recipe.image_sha256) {
            fs::remove_file(&partial)?;
            return Err(Error::new(format!(
                "{} does not match its pinned SHA-256",
                recipe.image_url
            )));
        }
        fs::rename(&partial, &file)?;
        Ok(file)
    }

    fn ensure_key(&self) -> Result<()> {
        if !self.key().exists() {
            fs::create_dir_all(&self.root)?;
            run(Command::new("ssh-keygen")
                .args(["-q", "-t", "ed25519", "-N", "", "-C", "rig", "-f"])
                .arg(self.key()))?;
        }
        Ok(())
    }

    // ---- guest lifecycle ----

    /// Starts the guest, or reuses the running one when it already shares
    /// `target`. Afterwards SSH answers, `target` is mounted read-only at
    /// `/mnt/target` and every head is plugged.
    pub fn up(&self, target: &Path) -> Result<()> {
        let target = target
            .canonicalize()
            .map_err(|e| Error::new(format!("{}: {e}", target.display())))?;
        let run_dir = self.run_dir();
        if self.is_up() {
            let shared = fs::read_to_string(run_dir.join("target")).unwrap_or_default();
            if Path::new(shared.trim()) == target {
                return Ok(());
            }
            eprintln!(
                "vm: the running guest shares {}, restarting it",
                shared.trim()
            );
        }
        self.down()?;
        require_kvm()?;
        let image = self.build_image()?;

        fs::create_dir_all(&run_dir)?;
        let disk = run_dir.join("disk.qcow2");
        overlay(&image, &disk)?;
        let port = free_port()?;

        let bus = run_dir.join("bus");
        background(
            Command::new("dbus-daemon").args([
                "--session",
                "--nofork",
                "--address",
                &self.bus_address(),
            ]),
            &run_dir,
            "dbus",
        )?;
        wait_for_file(&bus)?;

        let virtiofs_socket = run_dir.join("vfs.sock");
        let virtiofsd = find_virtiofsd()?;
        let mut daemon = Command::new(&virtiofsd);
        daemon
            .arg("--socket-path")
            .arg(&virtiofs_socket)
            .arg("--shared-dir")
            .arg(&target);
        daemon.args(["--sandbox", "none"]);
        let help = Command::new(&virtiofsd).arg("--help").output()?;
        if String::from_utf8_lossy(&help.stdout).contains("--readonly") {
            daemon.arg("--readonly");
        }
        background(&mut daemon, &run_dir, "virtiofsd")?;
        wait_for_file(&virtiofs_socket)?;

        let machine = Machine {
            disk,
            seed: None,
            ssh_port: port,
            heads: HEADS,
            memory_mb: MEMORY_MB,
            cpus: CPUS,
            qmp_socket: run_dir.join("qmp.sock"),
            serial_log: run_dir.join("serial.log"),
            virtiofs_socket: Some(virtiofs_socket),
            display_bus: Some(self.bus_address()),
            pid_file: Some(run_dir.join("qemu.pid")),
        };
        run(Command::new("qemu-system-x86_64").args(machine.arguments()))?;
        fs::write(run_dir.join("port"), port.to_string())?;
        fs::write(run_dir.join("target"), target.display().to_string())?;

        let deadline = Instant::now() + Duration::from_secs(180);
        while self.ssh("true").is_err() {
            if Instant::now() > deadline {
                return Err(Error::new(format!(
                    "the guest did not answer on SSH within 3 minutes, see {}",
                    run_dir.join("serial.log").display()
                )));
            }
            sleep(Duration::from_secs(1));
        }
        self.ssh(&format!(
            "mountpoint -q {TARGET_MOUNT} || sudo mount -t virtiofs -o ro {TARGET_TAG} {TARGET_MOUNT}"
        ))?;
        for head in 0..HEADS {
            self.plug_at_start(head)?;
        }
        Ok(())
    }

    /// A plug sent while the guest is still booting can get lost, and QEMU
    /// ignores a repeated one with the same size. So the connector is
    /// checked in the guest and the head is toggled until it shows up.
    fn plug_at_start(&self, head: u32) -> Result<()> {
        for _ in 0..5 {
            self.plug(head, DEFAULT_SIZE.0, DEFAULT_SIZE.1)?;
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                if self.connected(head)? {
                    return Ok(());
                }
                sleep(Duration::from_millis(200));
            }
            self.unplug(head)?;
            sleep(Duration::from_secs(2));
        }
        Err(Error::new(format!(
            "head {head} did not connect in the guest"
        )))
    }

    /// Stops the guest and its helpers. The overlay disk is thrown away, so
    /// the next start is the clean base image again.
    pub fn down(&self) -> Result<()> {
        let run_dir = self.run_dir();
        if let Ok(mut monitor) = self.monitor() {
            let _ = monitor.execute("quit", Value::Null);
            let deadline = Instant::now() + Duration::from_secs(10);
            while self.is_up() && Instant::now() < deadline {
                sleep(Duration::from_millis(100));
            }
        }
        for (name, program) in [("dbus", "dbus-daemon"), ("virtiofsd", "virtiofsd")] {
            stop_background(&run_dir, name, program);
        }
        if run_dir.exists() {
            fs::remove_dir_all(&run_dir)?;
        }
        Ok(())
    }

    // ---- guest access ----

    fn ssh_command(&self) -> Result<Command> {
        let port = fs::read_to_string(self.run_dir().join("port")).map_err(|_| {
            Error::new(format!(
                "no {} guest is running, start one with `cargo vm run`",
                self.family.name()
            ))
        })?;
        let mut ssh = Command::new("ssh");
        ssh.arg("-i").arg(self.key()).args(["-p", port.trim()]);
        for option in [
            "StrictHostKeyChecking=no",
            "UserKnownHostsFile=/dev/null",
            "LogLevel=ERROR",
            "BatchMode=yes",
            "ConnectTimeout=5",
        ] {
            ssh.args(["-o", option]);
        }
        ssh.arg(format!("{USER}@127.0.0.1"));
        Ok(ssh)
    }

    /// Runs a shell command in the guest as the rig user and returns what it printed.
    pub fn ssh(&self, command: &str) -> Result<String> {
        run(self.ssh_command()?.arg(command).stdin(Stdio::null()))
    }

    /// Runs a command in the guest attached to this terminal; without a
    /// command it is a login shell. Returns the exit code.
    pub fn ssh_attached(&self, command: &[String]) -> Result<i32> {
        let mut ssh = self.ssh_command()?;
        if command.is_empty() {
            ssh.arg("-t");
        }
        let status = ssh.args(command).status()?;
        Ok(status.code().unwrap_or(1))
    }

    /// Restarts the desktop session in the guest with `command` as the
    /// session's program. The command is a shell line run as the rig user in
    /// a logind session on seat0; a host-built binary is `/mnt/target/...`.
    pub fn session(&self, command: &str) -> Result<()> {
        let mut ssh = self.ssh_command()?;
        ssh.arg(format!(
            "sudo mkdir -p /run/rig && sudo tee /run/rig/session >/dev/null && date +%s | sudo tee /run/rig/since >/dev/null && sudo systemctl restart {SESSION_UNIT}"
        ));
        let mut child = ssh
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        child
            .stdin
            .take()
            .expect("piped")
            .write_all(format!("{command}\n").as_bytes())?;
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(Error::new(format!(
                "the session did not restart: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(())
    }

    /// Streams the session's log, from its last restart on, to this
    /// terminal until interrupted or until the guest goes away.
    pub fn follow_session(&self) -> Result<i32> {
        self.ssh_attached(&[format!(
            "sudo journalctl -f -o cat --since @$(cat /run/rig/since) {SESSION_LOG}"
        )])
    }

    /// The session's log since its last restart.
    pub fn session_log(&self) -> Result<String> {
        self.ssh(&format!(
            "sudo journalctl -o cat --no-pager --since @$(cat /run/rig/since) {SESSION_LOG}"
        ))
    }

    // ---- monitors ----

    /// Connects a head with the given size. The guest sees a DRM hot-plug event.
    pub fn plug(&self, head: u32, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(Error::new("a plugged head needs a size"));
        }
        self.set_head(head, width, height)
    }

    /// Disconnects a head. The guest sees a DRM hot-plug event.
    pub fn unplug(&self, head: u32) -> Result<()> {
        self.set_head(head, 0, 0)
    }

    /// Whether the guest's DRM connector of this head reports a monitor.
    /// Head 0 is the connector `Virtual-1`, head 1 is `Virtual-2`.
    pub fn connected(&self, head: u32) -> Result<bool> {
        let status = self.ssh(&format!(
            "cat /sys/class/drm/card*-Virtual-{}/status",
            head + 1
        ))?;
        Ok(status.trim() == "connected")
    }

    /// QEMU's monitor has no command for this; its D-Bus display interface
    /// has. A head counts as connected while a display reports a size for it.
    fn set_head(&self, head: u32, width: u32, height: u32) -> Result<()> {
        if head >= HEADS {
            return Err(Error::new(format!(
                "head {head} does not exist, the guest has {HEADS}"
            )));
        }
        run(Command::new("gdbus")
            .args([
                "call",
                "--address",
                &self.bus_address(),
                "--dest",
                "org.qemu",
            ])
            .args([
                "--object-path",
                &format!("/org/qemu/Display1/Console_{head}"),
            ])
            .args([
                "--method",
                "org.qemu.Display1.Console.SetUIInfo",
                "0",
                "0",
                "0",
                "0",
            ])
            .args([width.to_string(), height.to_string()]))?;
        Ok(())
    }

    /// Writes what a head shows to `file` as PNG, through QEMU's monitor.
    pub fn screenshot(&self, head: u32, file: &Path) -> Result<()> {
        let file = std::path::absolute(file)?;
        self.monitor()?.execute(
            "screendump",
            json!({ "filename": file, "format": "png", "device": GPU_ID, "head": head }),
        )?;
        Ok(())
    }
}

/// The workspace's target directory, as cargo reports it.
pub fn target_dir() -> Result<PathBuf> {
    let metadata =
        run(Command::new("cargo").args(["metadata", "--format-version", "1", "--no-deps"]))?;
    let metadata: Value = serde_json::from_str(&metadata)?;
    metadata["target_directory"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(|| Error::new("cargo metadata names no target directory"))
}

/// `cargo build` on the host, with cargo's own output on this terminal.
pub fn build(package: Option<&str>, release: bool) -> Result<()> {
    let mut cargo = Command::new("cargo");
    cargo.arg("build");
    match package {
        Some(package) => cargo.args(["--package", package]),
        None => cargo.arg("--workspace"),
    };
    if release {
        cargo.arg("--release");
    }
    if !cargo.status()?.success() {
        return Err(Error::new("the build failed"));
    }
    Ok(())
}

fn require_kvm() -> Result<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .map(drop)
        .map_err(|e| {
            Error::new(format!(
                "this user cannot open /dev/kvm ({e}); the rig needs KVM"
            ))
        })
}

fn overlay(base: &Path, disk: &Path) -> Result<()> {
    run(Command::new("qemu-img")
        .args(["create", "-q", "-f", "qcow2", "-F", "qcow2", "-b"])
        .arg(base)
        .arg(disk)
        .arg(DISK_SIZE))?;
    Ok(())
}

fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

fn find_virtiofsd() -> Result<PathBuf> {
    if let Ok(found) = run(Command::new("sh").args(["-c", "command -v virtiofsd"])) {
        return Ok(PathBuf::from(found.trim()));
    }
    [
        "/usr/libexec/virtiofsd",
        "/usr/lib/qemu/virtiofsd",
        "/usr/lib/virtiofsd",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.exists())
    .ok_or_else(|| Error::new("virtiofsd is not installed"))
}

/// Starts a helper that outlives this process: its own process group, so a
/// Ctrl-C on the log stream does not take the guest down with it.
fn background(command: &mut Command, run_dir: &Path, name: &str) -> Result<()> {
    let log = File::create(run_dir.join(format!("{name}.log")))?;
    let program = command.get_program().to_string_lossy().into_owned();
    let child = command
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .process_group(0)
        .spawn()
        .map_err(|e| Error::new(format!("{program}: {e}")))?;
    fs::write(run_dir.join(format!("{name}.pid")), child.id().to_string())?;
    Ok(())
}

/// Stops a helper started by [`background`], but only when the pid still
/// belongs to that program.
fn stop_background(run_dir: &Path, name: &str, program: &str) {
    let Ok(pid) = fs::read_to_string(run_dir.join(format!("{name}.pid"))) else {
        return;
    };
    let pid = pid.trim();
    let running = fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
    if running.trim() == program {
        let _ = Command::new("kill").arg(pid).status();
    }
}

fn wait_for_file(path: &Path) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        if Instant::now() > deadline {
            return Err(Error::new(format!("{} did not appear", path.display())));
        }
        sleep(Duration::from_millis(50));
    }
    Ok(())
}

/// Runs a tool to its end and returns its standard output; a failure
/// carries the tool's name and what it wrote to standard error.
fn run(command: &mut Command) -> Result<String> {
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command
        .output()
        .map_err(|e| Error::new(format!("{program}: {e}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::new(format!("{program} failed: {}", stderr.trim())));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}
