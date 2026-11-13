# Decisions and open questions

Every decision taken and every open question, each with its date and source issue. Where two issues disagree, the disagreement is listed under [Contradictions between issues](#contradictions-between-issues) and is not resolved here.

"2026-11-10/11" is the planning thread the spec issues were written from.

## Decisions taken

| Date | Decision | Source |
|---|---|---|
| 2026-11-04 | Every bit of logic is defined and approved before it goes in; no logic is assumed from the old code. | #V-1 |
| 2026-11-10/11 | Rust for everything. Lean only as a model for a few small kernels (the checker rules), not as the implementation language. | #RUST-18 |
| 2026-11-10/11 | Composition is data (a JSON graph plus terminating expressions). Rust is for primitives; WASM for third-party logic. | #RUST-18 |
| 2026-11-10/11 | A2UI stays the view format; ports and wires are a manifest above it. | #RUST-18 |
| 2026-11-10/11 | No GTK or Qt compatibility layer: generated themes, portals, compositor decorations. | #RUST-18, #RUST-4 |
| 2026-11-10/11 | Mint tools are rebuilt by behaviour, never from their code. | #RUST-18, #RUST-5 |
| 2026-11-10/11 | A team is offline when none of its full devices runs. | #RUST-18, #RUST-3 |
| 2026-11-10/11 | New crates, plain functional names. No `e*` prefix, no `rerusted-*` prefix. | #RUST-6 |
| 2026-11-10/11 | The crate called "core" in the planning thread is named `base`, because a crate named `core` shadows Rust's own. | #RUST-6 |
| 2026-11-10/11 | One crate does one thing, depends only downward, and is usable alone. | #RUST-6 |
| 2026-11-10/11 | No app, no UI painter, no network in phase 0. | #RUST-6 |
| 2026-11-10/11 | Shared scaffold: workspace with `resolver = "3"`, `members = ["crates/*"]`, edition 2024, `publish = false`, `unsafe_code = "forbid"`; `rust-toolchain.toml` on `stable`; `.gitignore` with `/target`. | #RUST-6 |
| 2026-11-11 | Accent A: `oklch(0.56 0.15 42)` (`#ba4f1f`), chosen over a redder and a browner variant. | #RUST-15 |
| 2026-11-11 | Dark only for this phase; a light theme is derived later from the same hue. | #RUST-15 |
| 2026-11-11 | The token `accent` is the rust colour, `accentForeground` = foreground. Exponential's `accent` hover role moves to `muted`. | #RUST-15 |
| 2026-11-11 | Tokens not in the #RUST-5 table: `primary` = `oklch(0.922 0.008 50)`, `primaryForeground` = card, `input` = border `oklch(0.42 0.04 36)`, `ring` = accent, `destructive` = Exponential's `oklch(0.704 0.191 22.216)`. | #RUST-15 |
| 2026-11-11 | The accent is checked against WCAG AA 3:1 (UI components and large text), not 4.5:1; it is never used for body text. | #RUST-15 |
| 2026-11-11 | The token generator (OKLCH to sRGB hex, contrast check, logo and icons) is a Rust tool in `design/tools/`, its own cargo root. | #RUST-15 |
| 2026-11-11 | Taken from Exponential's tokens unchanged: radius, size, spacing, type. Left out: semantic, avatar, glass, diff, menu, transcript, sidebar, chat, motion. | #RUST-15 |
| 2026-11-13 | Dev and test rig in four tiers: nested, headless, VM, real hardware. Code and compile on the host only. Spec: `docs/spec/08-dev-rig.md`. | #RUST-21 |
| 2026-11-13 | Reference machines: the development machine (i9-13900K, RTX 4090 and UHD 770, Linux Mint 22.3) and a second one named later. | #RUST-21 |
| 2026-11-13 | Screenshot tests render in software in CI with a pinned Mesa: exact match for what the compositor draws, a small tolerance for gpui client content. GPU comparisons, with a tolerance, on tier 4 only. | #RUST-21 |
| 2026-11-13 | CI runs on GitHub Actions until #RUST-9 exists: format, clippy with warnings as errors, tests on Linux and macOS. The VM does not run in CI. | #RUST-21 |
| 2026-11-13 | The VM runner is the crate `vm`, a library and a binary, started as `cargo vm` through a cargo alias. It depends on no workspace crate, uses no `unsafe`, and shells out to QEMU, virtiofsd, ssh, dbus-daemon and gdbus instead of linking them. | #RUST-22 |
| 2026-11-13 | Guest for the apt family and for running host-built binaries: the Ubuntu 24.04 cloud image, the base of the host's Linux Mint 22.3. Not a real Mint guest, which would need root or a scripted ISO install. | #RUST-22 |
| 2026-11-13 | Base images are built from official cloud images pinned by dated URL and SHA-256, provisioned once by cloud-init from a pinned package source, without root on the host and without a compiler in the guest. One per family: Ubuntu 24.04 (apt), Fedora 45 (rpm), Arch Linux (pacman). | #RUST-22 |
| 2026-11-13 | The session in the guest is a direct launch: a systemd unit that opens a logind session on seat0, tty7 through PAM. No login screen; that entry belongs to packaging. | #RUST-22 |
| 2026-11-13 | Monitor hot-plug goes through QEMU's D-Bus display interface (`SetUIInfo`), because QEMU's monitor has no command for it. The monitor is used for lifecycle and screenshots. The fallback of adding and removing a second GPU through the monitor was not needed and is not built. | #RUST-22 |
| 2026-11-13 | The binary the runner launches is a parameter (`--bin`, `--session`). Until #RUST-23 has a compositor the default is sway. | #RUST-22 |
| 2026-11-13 | Rig images and state live in `~/.cache/rerusted/vm`, outside the checkout, so worktrees share them. Each guest start uses a throwaway overlay on the base image. | #RUST-22 |
| 2026-11-13 | `target/` is shared by virtiofs and mounted read-only by the guest. The host side enforces it only when the installed virtiofsd has `--readonly`; version 1.10 on the development machine does not. | #RUST-22 |

## Open questions

| Date | Question | Source |
|---|---|---|
| 2026-11-10/11 | p2panda crates (auth, encryption, spaces) behind our own trait, or our own signed log? Proposed: a spike with two devices and a phone (remove a member, rejoin offline) before deciding. | #RUST-3, #RUST-18 |
| 2026-11-10/11 | Painter: own on Vello, zgui, or gpui. | #RUST-18 (#RUST-8) |
| 2026-11-10/11 | CI runner: write it ourselves, or build on an existing Rust runner (chimera) and write only the planner? | #RUST-9, #RUST-18 |
| 2026-11-10/11 | Licence: Apache-2.0 is assumed. Ask before the first merge if it should differ. | #RUST-6, #RUST-18 |
| 2026-11-10/11 | Base system for the image. | #RUST-18 (#RUST-12) |
| 2026-11-10/11 | Label granularity: per provider (`twitter`) or per account and kind? | #RUST-2 |
| 2026-11-10/11 | Is `allow` per wire enough, or does it need an expiry? | #RUST-2 |
| 2026-11-10/11 | Dynamic children (one subtree per list item): template in the `view` primitive only, or a `for-each` in compose? | #RUST-2 |
| 2026-11-10/11 | Concurrent admin actions need one fixed rule (proposal: the owner wins, then seniority in the log). | #RUST-3 |
| 2026-11-10/11 | Rented devices hold the team key and are trusted members; the end-to-end claim has to be worded accordingly. | #RUST-3 |
| 2026-11-10/11 | Recovery when every device is lost and the words too: none. Is that acceptable? | #RUST-3 |
| 2026-11-10/11 | Is `process` one primitive, or `process` + a separate `window` view primitive? | #RUST-4 |
| 2026-11-10/11 | Which interfaces are in the first cut besides `tasks`, `code`, `ci`, `llm`? | #RUST-4 |
| 2026-11-10/11 | Each `system.*` provider enlarges the trusted base. How are they reviewed? | #RUST-4 |
| 2026-11-10/11 | Which settings pages are in the first usable cut? Proposal: display, sound, network, keyboard, languages, themes. | #RUST-5 |
| 2026-11-10/11 | File manager: keep Nemo first (as decided for the earlier desktop work) or wrap a Rust one? | #RUST-5 |
| 2026-11-10/11 | "Whose main is main" without a server needs one rule (delegates and a threshold, as Radicle does). | #RUST-9 |
| 2026-11-13 | Which machine is the second reference machine? | #RUST-21, #RUST-19 |
| 2026-11-13 | The commands of tier 1 (nested) and tier 2 (headless) come from the compositor spike and are not in the spec yet. | #RUST-21, #RUST-23 |

Answered: "Light theme now or later?" (#RUST-5) was answered on 2026-11-11 in #RUST-15: dark only for now.

## Contradictions between issues

Listed, not resolved.

1. **Teams.** #V-1 (2026-11-04): "we dont want the concept of a team in there". #RUST-3 and #RUST-18 (2026-11-10/11) build on teams: the member log, the team key, invites and the `team` scope.
2. **Server.** #V-1 (2026-11-04): "exponential will get multiplayer and also gets a server". #RUST-18 (2026-11-10/11): "Everything p2p; no server holds state or authority."
3. **Agents.** #RUST-18 (2026-11-10/11), principle 6: "Only sessions: no separate agent concept." #RUST-2 and #RUST-4 (2026-11-10/11) have an `agent` primitive, and #V-1 (2026-11-04) lists an eAgent library.
4. **`code` means three things.** The `code` module in #RUST-18, the `code` primitive (a WASM component) in #RUST-4 and #RUST-2, and the `code` interface (repos) in #RUST-4 and #RUST-9 (all 2026-11-10/11).
5. **Missing reference.** #RUST-9 (2026-11-10/11) defines the Forge events as "the list in #NEW-36", an issue that is not on this board.
6. **Licence.** #RUST-6 (2026-11-10/11) assumes Apache-2.0 "as for the earlier desktop work"; it is not decided (#RUST-18 open decision 4).
