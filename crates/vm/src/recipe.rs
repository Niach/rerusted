//! Base image recipes: one pinned cloud image per distro family, plus the
//! cloud-init seed that turns it into a rig guest.

use crate::error::{Error, Result};

/// User the session runs as in every guest.
pub const USER: &str = "rig";
/// Where the host's `target/` appears in the guest, read-only.
pub const TARGET_MOUNT: &str = "/mnt/target";
/// The systemd unit that holds the desktop session.
pub const SESSION_UNIT: &str = "rig-session";
/// Stock compositor that stands in until the workspace has its own binary.
pub const STAND_IN_SESSION: &str = "sway -c /etc/rig/sway.config";
/// Written to the serial console when provisioning finished without error.
pub const PROVISIONED_MARK: &str = "RIG-PROVISIONED";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Apt,
    Rpm,
    Pacman,
}

impl Family {
    pub const ALL: [Family; 3] = [Family::Apt, Family::Rpm, Family::Pacman];

    pub fn name(self) -> &'static str {
        match self {
            Family::Apt => "apt",
            Family::Rpm => "rpm",
            Family::Pacman => "pacman",
        }
    }

    pub fn parse(name: &str) -> Result<Self> {
        Self::ALL
            .into_iter()
            .find(|family| family.name() == name)
            .ok_or_else(|| {
                Error::new(format!(
                    "unknown family '{name}', expected apt, rpm or pacman"
                ))
            })
    }
}

pub struct Recipe {
    pub family: Family,
    pub distro: &'static str,
    pub image_url: &'static str,
    pub image_sha256: &'static str,
    /// Shell lines that install the packages from a pinned package source.
    pub install: &'static str,
}

pub fn recipe(family: Family) -> Recipe {
    match family {
        // Same base as the host (Linux Mint 22.x), so host-built binaries run unchanged.
        Family::Apt => Recipe {
            family,
            distro: "Ubuntu 24.04 (noble), cloud image 20261017",
            image_url: "https://cloud-images.ubuntu.com/noble/20261017/noble-server-cloudimg-amd64.img",
            image_sha256: "04e8f2461a16a44e78a370284471f29616cdd9baf1893df25807b71f985b6c94",
            install: r#"echo 'APT::Snapshot "20261017T000000Z";' > /etc/apt/apt.conf.d/50snapshot
export DEBIAN_FRONTEND=noninteractive
apt-get update
apt-get install -y --no-install-recommends sway foot xwayland kbd dbus-user-session fonts-dejavu-core libgl1-mesa-dri libegl1 libgles2 libgbm1 mesa-vulkan-drivers libseat1 libinput10 libxkbcommon0 libwayland-server0 libdrm2"#,
        },
        // Only the release repository, which is frozen; the moving updates repository is left out.
        Family::Rpm => Recipe {
            family,
            distro: "Fedora 45, cloud image 45-1.4",
            image_url: "https://dl.fedoraproject.org/pub/fedora/linux/releases/45/Cloud/x86_64/images/Fedora-Cloud-Base-Generic-45-1.4.x86_64.qcow2",
            image_sha256: "50bdc5e70f6a24f540932d15411c1c6ae9fcfa40fb990d876d83c5cc823b2ad0",
            install: "dnf -y --repo=fedora install sway foot xorg-x11-server-Xwayland kbd dejavu-sans-mono-fonts mesa-dri-drivers mesa-libEGL mesa-libgbm mesa-vulkan-drivers",
        },
        // Packages come from the Arch Linux Archive at the image's date.
        Family::Pacman => Recipe {
            family,
            distro: "Arch Linux, cloud image 20261101",
            image_url: "https://geo.mirror.pkgbuild.com/images/v20261101.620452/Arch-Linux-x86_64-cloudimg-20261101.620452.qcow2",
            image_sha256: "794eb4889b39e1f0ed7d5ae998b2a1ed2137db8cf8c3476cf2962c7437ecb003",
            install: r#"echo 'Server = https://archive.archlinux.org/repos/2026/11/01/$repo/os/$arch' > /etc/pacman.d/mirrorlist
pacman -Syyuu --noconfirm --needed sway foot xorg-xwayland kbd ttf-dejavu mesa vulkan-swrast"#,
        },
    }
}

/// A real logind session on seat0 without a login screen: PAM opens the
/// session on tty7 and the unit executes whatever `/run/rig/session` holds.
const SESSION_SERVICE: &str = "\
[Unit]
Description=Rig desktop session
After=systemd-user-sessions.service systemd-logind.service
Conflicts=getty@tty7.service

[Service]
User=rig
PAMName=login
TTYPath=/dev/tty7
TTYReset=yes
TTYVHangup=yes
TTYVTDisallocate=yes
StandardInput=tty-fail
StandardOutput=journal
StandardError=journal
UtmpIdentifier=tty7
UtmpMode=user
Environment=XDG_SESSION_TYPE=wayland XDG_SEAT=seat0 XDG_VTNR=7
ExecStartPre=+/usr/bin/chvt 7
ExecStart=/usr/local/bin/rig-session
";

const SESSION_LAUNCHER: &str = "\
#!/bin/sh
exec sh -c \"$(cat /run/rig/session)\"
";

/// Both virtual monitors report the same EDID, so the stand-in is told by
/// connector name which workspace belongs to which output.
const STAND_IN_CONFIG: &str = "\
workspace 1 output Virtual-1
workspace 2 output Virtual-2
output * bg #ba4f1f solid_color
";

impl Recipe {
    pub fn meta_data(&self) -> String {
        format!(
            "instance-id: rig-{}\nlocal-hostname: rig-{}\n",
            self.family.name(),
            self.family.name()
        )
    }

    /// The cloud-init user data. Provisioning ends with a power-off; the
    /// result is the base image.
    pub fn user_data(&self, public_key: &str) -> String {
        let provision = format!(
            "#!/bin/sh\nset -eux\n{install}\nmkdir -p {TARGET_MOUNT}\ntouch /etc/cloud/cloud-init.disabled\necho {PROVISIONED_MARK} > /dev/ttyS0\n",
            install = self.install,
        );
        let mut out = String::from("#cloud-config\n");
        out.push_str(&format!(
            "users:\n  - name: {USER}\n    shell: /bin/bash\n    sudo: \"ALL=(ALL) NOPASSWD:ALL\"\n    lock_passwd: true\n    ssh_authorized_keys:\n      - {}\n",
            public_key.trim()
        ));
        out.push_str("ssh_pwauth: false\nwrite_files:\n");
        for (path, mode, content) in [
            (
                "/etc/systemd/system/rig-session.service",
                "0644",
                SESSION_SERVICE,
            ),
            ("/usr/local/bin/rig-session", "0755", SESSION_LAUNCHER),
            ("/etc/rig/sway.config", "0644", STAND_IN_CONFIG),
            ("/usr/local/sbin/rig-provision", "0755", provision.as_str()),
        ] {
            out.push_str(&format!(
                "  - path: {path}\n    permissions: \"{mode}\"\n    content: |\n"
            ));
            for line in content.lines() {
                if line.is_empty() {
                    out.push('\n');
                } else {
                    out.push_str(&format!("      {line}\n"));
                }
            }
        }
        out.push_str("runcmd:\n  - [/usr/local/sbin/rig-provision]\npower_state:\n  mode: poweroff\n  timeout: 30\n");
        out
    }

    /// Changes whenever the recipe changes, so a stale base image is rebuilt.
    /// The SSH key is left out: it does not change what is installed.
    pub fn fingerprint(&self) -> String {
        let text = format!(
            "{}\n{}\n{}",
            self.image_sha256,
            self.user_data(""),
            self.meta_data()
        );
        // FNV-1a, 64 bit.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in text.bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        format!("{hash:016x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn family_names_round_trip() {
        for family in Family::ALL {
            assert_eq!(Family::parse(family.name()).unwrap(), family);
        }
        assert!(Family::parse("nix").is_err());
    }

    #[test]
    fn user_data_carries_key_unit_and_install_lines() {
        let recipe = recipe(Family::Apt);
        let data = recipe.user_data("ssh-ed25519 AAAA rig\n");
        assert!(data.starts_with("#cloud-config\n"));
        assert!(data.contains("      - ssh-ed25519 AAAA rig\n"));
        assert!(data.contains("      PAMName=login\n"));
        assert!(data.contains("      apt-get update\n"));
        assert!(data.contains("      echo RIG-PROVISIONED > /dev/ttyS0\n"));
        assert!(data.ends_with("power_state:\n  mode: poweroff\n  timeout: 30\n"));
    }

    #[test]
    fn no_guest_installs_a_compiler() {
        for family in Family::ALL {
            let install = recipe(family).install;
            for word in [
                "gcc",
                "clang",
                "rustc",
                "cargo",
                "build-essential",
                "base-devel",
            ] {
                assert!(!install.contains(word), "{} installs {word}", family.name());
            }
        }
    }

    #[test]
    fn every_image_is_pinned_and_fingerprints_differ() {
        let prints: Vec<String> = Family::ALL
            .into_iter()
            .map(|f| recipe(f).fingerprint())
            .collect();
        for family in Family::ALL {
            let recipe = recipe(family);
            assert_eq!(recipe.image_sha256.len(), 64);
            assert!(
                !recipe.image_url.contains("/current/") && !recipe.image_url.contains("latest")
            );
            assert_eq!(recipe.fingerprint(), recipe.fingerprint());
        }
        assert!(prints[0] != prints[1] && prints[1] != prints[2] && prints[0] != prints[2]);
    }
}
