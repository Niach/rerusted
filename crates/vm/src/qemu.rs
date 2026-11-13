//! The QEMU command line of a rig guest.

use std::path::PathBuf;

/// QOM id of the display device, used for per-head screenshots.
pub const GPU_ID: &str = "gpu0";
/// virtiofs tag under which the guest finds the host's `target/`.
pub const TARGET_TAG: &str = "target";

pub struct Machine {
    pub disk: PathBuf,
    /// Directory served as the cloud-init seed; only for the image build.
    pub seed: Option<PathBuf>,
    pub ssh_port: u16,
    pub heads: u32,
    pub memory_mb: u32,
    pub cpus: u32,
    pub qmp_socket: PathBuf,
    pub serial_log: PathBuf,
    /// vhost-user socket of the virtiofs daemon that shares `target/`.
    pub virtiofs_socket: Option<PathBuf>,
    /// Address of the private bus that carries QEMU's display interface.
    pub display_bus: Option<String>,
    /// With a pid file QEMU goes to the background; without one it stays
    /// in the foreground and does not reboot (the image build).
    pub pid_file: Option<PathBuf>,
}

impl Machine {
    pub fn arguments(&self) -> Vec<String> {
        let memory = format!("{}M", self.memory_mb);
        let mut args: Vec<String> = Vec::new();
        let mut add = |items: &[&str]| args.extend(items.iter().map(|s| s.to_string()));

        add(&["-machine", "q35,accel=kvm", "-cpu", "host", "-nodefaults"]);
        add(&["-smp", &self.cpus.to_string(), "-m", &memory]);
        // Shared memory is what lets the virtiofs daemon see guest pages.
        add(&[
            "-object",
            &format!("memory-backend-memfd,id=mem,size={memory},share=on"),
        ]);
        add(&["-numa", "node,memdev=mem"]);
        add(&[
            "-drive",
            &format!("file={},if=virtio,format=qcow2", self.disk.display()),
        ]);
        if let Some(seed) = &self.seed {
            add(&[
                "-drive",
                &format!(
                    "file.driver=vvfat,file.dir={},file.label=cidata,file.rw=off,format=raw,if=virtio,readonly=on",
                    seed.display()
                ),
            ]);
        }
        add(&[
            "-nic",
            &format!(
                "user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:{}-:22",
                self.ssh_port
            ),
        ]);
        add(&[
            "-device",
            &format!("virtio-vga,id={GPU_ID},max_outputs={}", self.heads),
        ]);
        add(&[
            "-device",
            "virtio-keyboard-pci",
            "-device",
            "virtio-tablet-pci",
        ]);
        if let Some(socket) = &self.virtiofs_socket {
            add(&[
                "-chardev",
                &format!("socket,id=vfs,path={}", socket.display()),
            ]);
            add(&[
                "-device",
                &format!("vhost-user-fs-pci,chardev=vfs,tag={TARGET_TAG}"),
            ]);
        }
        match &self.display_bus {
            Some(bus) => add(&["-display", &format!("dbus,addr={bus}")]),
            None => add(&["-display", "none"]),
        }
        add(&[
            "-qmp",
            &format!("unix:{},server=on,wait=off", self.qmp_socket.display()),
        ]);
        add(&["-serial", &format!("file:{}", self.serial_log.display())]);
        match &self.pid_file {
            Some(pid_file) => add(&["-daemonize", "-pidfile", &pid_file.display().to_string()]),
            None => add(&["-no-reboot"]),
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn machine() -> Machine {
        Machine {
            disk: "/r/disk.qcow2".into(),
            seed: None,
            ssh_port: 2222,
            heads: 2,
            memory_mb: 4096,
            cpus: 4,
            qmp_socket: "/r/qmp.sock".into(),
            serial_log: "/r/serial.log".into(),
            virtiofs_socket: Some("/r/vfs.sock".into()),
            display_bus: Some("unix:path=/r/bus".into()),
            pid_file: Some("/r/qemu.pid".into()),
        }
    }

    fn has(args: &[String], flag: &str, value: &str) -> bool {
        args.windows(2)
            .any(|pair| pair[0] == flag && pair[1] == value)
    }

    #[test]
    fn a_running_guest_has_two_heads_kvm_virtiofs_and_the_display_bus() {
        let args = machine().arguments();
        assert!(has(&args, "-machine", "q35,accel=kvm"));
        assert!(has(&args, "-device", "virtio-vga,id=gpu0,max_outputs=2"));
        assert!(has(
            &args,
            "-device",
            "vhost-user-fs-pci,chardev=vfs,tag=target"
        ));
        assert!(has(
            &args,
            "-object",
            "memory-backend-memfd,id=mem,size=4096M,share=on"
        ));
        assert!(has(&args, "-display", "dbus,addr=unix:path=/r/bus"));
        assert!(has(
            &args,
            "-nic",
            "user,model=virtio-net-pci,hostfwd=tcp:127.0.0.1:2222-:22"
        ));
        assert!(has(&args, "-pidfile", "/r/qemu.pid"));
        assert!(
            !args
                .iter()
                .any(|a| a.contains("cidata") || a == "-no-reboot")
        );
    }

    #[test]
    fn an_image_build_has_the_seed_and_stays_in_the_foreground() {
        let build = Machine {
            seed: Some("/b/seed".into()),
            virtiofs_socket: None,
            display_bus: None,
            pid_file: None,
            ..machine()
        };
        let args = build.arguments();
        assert!(
            args.iter()
                .any(|a| a.contains("file.dir=/b/seed,file.label=cidata"))
        );
        assert!(has(&args, "-display", "none"));
        assert!(args.contains(&"-no-reboot".to_string()));
        assert!(
            !args
                .iter()
                .any(|a| a == "-daemonize" || a.contains("vhost-user-fs"))
        );
    }
}
