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
| 2026-11-13 | Licence: Apache-2.0. The workspace manifest says so and the text is in `LICENSE`. | #RUST-6, #RUST-31 |
| 2026-11-13 | The compositor library is Smithay. | #RUST-20 |
| 2026-11-13 | The painter is gpui. | #RUST-8 |
| 2026-11-13 | `unsafe` rule: `forbid` is the workspace default; a crate that needs it overrides it to `deny` in its own manifest, is listed here by name with the reason, keeps `unsafe` in one module with a safety comment per block, and exposes only a safe API. `base`, `graph`, `identity` and `store` never get the exception. | #RUST-6 |
| 2026-11-13 | Crates with the `unsafe` exception: none. The compositor spike (`compositor`, `spikes/gpui-shell`) needed none of its own. | #RUST-23 |
| 2026-11-13 | The spike crate is `compositor`. It depends on no other workspace crate and is Linux only; elsewhere it compiles to nothing, so `cargo test` stays green on macOS. | #RUST-23 |
| 2026-11-13 | The model type is `Desk`: one seat plus the outputs it works on, with its own window stack, Wayland socket and optional XWayland. The person's desk and an agent space are the same struct; a space is a desk with one virtual output. | #RUST-23 |
| 2026-11-13 | A client belongs to the desk whose socket it connected to, and sees only that desk's seat and outputs. Layer shell is offered on the person's desk only. | #RUST-23 |
| 2026-11-13 | Smithay is pinned to 0.7.0 plus one commit of our own (client filters for the `wl_seat` and `wl_output` globals), taken from a fork by git revision. Smithay has no such filter, in 0.7.0 or on `master`; the commit is to be sent upstream. | #RUST-23 |
| 2026-11-13 | Smithay features in use: `wayland_frontend`, `desktop`, `renderer_pixman`, `renderer_gl`, `backend_winit`, `xwayland`. No DRM, GBM, libinput, udev or libseat in the spike. | #RUST-23 |
| 2026-11-13 | Headless renders with pixman in memory (no GPU, shared-memory clients only); nested renders with GLES through winit. Every output, shown or not, renders into an off-screen target of its own; the nested window shows those textures. | #RUST-23 |
| 2026-11-13 | Rendering is driven by client commits, at most once per 16.7 ms per output, never by a timer. A frame is delivered only when the render had damage. | #RUST-23 |
| 2026-11-13 | One XWayland per space, started with the space. The person's desk has none in the spike. | #RUST-23 |
| 2026-11-13 | A space is driven through a Rust handle in the same process (`DeskHandle`: spawn, pointer, button, key, text, windows, frames). No wire protocol for agents in the spike. | #RUST-23 |
| 2026-11-13 | Windows in a space fill its output; windows on the person's desk cascade on the output the pointer is on and are kept out from under panels. No decorations, no layout. | #RUST-23 |
| 2026-11-13 | A new window gets the keyboard of its desk; a click gives it to the window under the pointer. | #RUST-23 |
| 2026-11-13 | Injected text is typed on a fixed US keymap, independent of the host's layout. | #RUST-23 |
| 2026-11-13 | The test client for the compositor lives in `crates/compositor/tests/common/` and runs on a thread of the test; it is not a crate. | #RUST-23 |
| 2026-11-13 | gpui for the layer-shell check is zed `main` at a pinned git revision, in `spikes/gpui-shell/`, a cargo root of its own so the workspace never builds zed. gpui 0.2.2 on crates.io has no layer shell. | #RUST-23 |
| 2026-11-13 | The CI workflow installs what the compositor needs on Linux (Wayland, xkbcommon, pixman and EGL headers; XWayland, xterm, zenity and dbus-daemon for the tests) and has one more job that runs the gpui client against the headless compositor on software Vulkan. | #RUST-23, #RUST-21 |
| 2026-11-13 | Spike crates: `teamlog` (our trait and types, no dependency) and `teamlog-p2panda` (the adapter). `identity` stays reserved for #RUST-17. Neither needs `unsafe`. | #RUST-30 |
| 2026-11-13 | `rust-toolchain.toml` stays on `stable`; the host's stable was updated to 1.100.0 because p2panda 0.8 needs Rust 1.96. | #RUST-30 |
| 2026-11-13 | The spike's transport is plain iroh 1.4 with a small exchange written in the test (heights, then what the other lacks). p2panda-net is not judged. iroh is a dev-dependency of the adapter only. | #RUST-30 |
| 2026-11-13 | The spike's tests run in the `ci` workflow from #RUST-21 (format, clippy, tests on Linux and macOS); the spike's own one-job `test` workflow was dropped when merging `main`. | #RUST-30 |
| 2026-11-13 | Spike mapping: one member is one device key; `Role::Admin` is p2panda's `manage`, `Role::Member` its `write`; a team is one p2panda space. | #RUST-30 |
| 2026-11-13 | An entry is a p2panda-core header plus the spaces message as its body, not in the header's extensions: the header decoder refuses any field longer than 512. `Entry.version` (1) is ours and wraps both. | #RUST-30 |
| 2026-11-13 | The trait's history is `Payload { author, n, data }` instead of the planned entry id per payload: p2panda's decrypt event names neither the entry nor the author, so the adapter puts a versioned envelope (author, counter) inside the ciphertext. | #RUST-30 |
| 2026-11-13 | An entry p2panda refuses is kept, counts as tried for the entries that depend on it, and is tried again whenever another entry was applied. Holding it back would hold back every later entry. | #RUST-30 |
| 2026-11-13 | The adapter keeps its own tables next to p2panda's (log index with state, decrypted payloads, credentials): p2panda's store cannot list the devices it holds logs of and keeps no plaintext. | #RUST-30 |
| 2026-11-13 | Where p2panda 0.8 behaves differently from #RUST-3, the tests assert what it does and mark it FINDING; they are not bent to pass. | #RUST-30 |
| 2026-11-14 | Only the owner removes or demotes an admin. Ranks are owner, admin, member; an operation counts only if its author ranks strictly higher than its target and than the role it hands out. The owner is the device that created the team, until it hands the team over; nobody can remove the owner. This replaces "the owner wins, then seniority". | #RUST-3 |
| 2026-11-14 | A member that is admitted again does not read what was written while it was out: it gets a new key and none of the old ones. A device new to the team gets every key and reads the whole history. | #RUST-3 |
| 2026-11-14 | What a member wrote that its removal had not seen is dropped: void on every replica, its own included. A replica that had shown it is told (`Event::Dropped`). | #RUST-3 |
| 2026-11-14 | One-week spike of our own team log in `identity`, behind the `teamlog` trait, after the research on what could replace p2panda. One member is one device key, as in #RUST-30; user keys, device certificates, invites and the 24 words are not in it. | #RUST-3, #RUST-17 |
| 2026-11-14 | A removal, an admission and a change of role name a cut: how many entries of the target's log the author held. The new standing applies from the cut on. Every entry names the operation its author acts under, so entries written while removed stay void after a new admission. | #RUST-17 |
| 2026-11-14 | Concurrent operations: the owner's always count; an admin's operation on a device is void when the owner acted on the same device and neither had seen the other; between admins a removal beats an admission it had not seen. | #RUST-17 |
| 2026-11-14 | Team key: 32 random bytes named by their hash, sealed per device with HPKE (RFC 9180: X25519, HKDF-SHA256, ChaCha20-Poly1305, crate `hpke`), payloads under XChaCha20-Poly1305. A removal carries the new key in the same entry. The current key is the newest one held by exactly the current members, read from the resolved member list, never from a timestamp; when there is none, the next writer makes one. | #RUST-17 |
| 2026-11-14 | A key counts as held by every device it was sealed to, also by an operation that is void: an overruled admission has handed the key over all the same. Found by the random test, which had an outsider reading. | #RUST-17 |
| 2026-11-14 | Entry format version 1: hand-written, big-endian, one canonical encoding, the version byte first and inside the signature; Ed25519 over a context string and the body; the id is SHA-256 of body and signature. `Role::Owner`, `Event::Dropped`, `hand_over`, `leave`, `tips` and the three fork methods are added to the `teamlog` trait; a team's id is the id of its first entry. | #RUST-17, #RUST-29 |
| 2026-11-14 | `crates/teamlog-p2panda/tests/strict_rank.rs` stays as the record of the comparison: the same rank rule as a `p2panda-auth` 0.8.0 resolver. It passes, through a workaround, and is not a path we take. | #RUST-3 |
| 2026-11-14 | A device that signs two entries for one position of its log has forked. From the lowest such position on, nothing of its log counts, on any branch; what it wrote before stays; the device is out of every team for good and joins again as a new device. Chosen because it needs no vote and no clock: two signed entries are the whole proof, anybody can check it, and no rule has to pick a branch that the forker could then aim for. | #RUST-39 |
| 2026-11-14 | A fork's proof (the two entries) travels beside the logs, not in one: a log cannot carry the proof that it is forked itself. A replica holds and offers a forked log only up to the fork. Replicas compare the last entry of each log, not only its length; where they differ, one sends the log from the start and the other finds where they part. | #RUST-39 |
| 2026-11-14 | An operation names, for every device, the newest operation its author held (device, position, id), not only the newest ones overall. A name that points into the void part of a forked log reads as the last operation of that log that counts, so an operation built on a forker's void one keeps its place and its ancestry. | #RUST-39 |
| 2026-11-14 | A device key must never be restored from a backup and used again: writing from an old position is a fork. A restored device that syncs before it writes gets its own entries back and does not fork. | #RUST-39 |
| 2026-11-14 | The owner hands the team over with one operation. It takes effect at its own position in the owner's log: what the old owner writes after it is an admin's, and the old owner stays as admin. An owner has one log, so two handovers "at once" can only be a fork. A handover beats an admin's removal of the new owner that had not seen it. | #RUST-39 |
| 2026-11-14 | The new owner accepts with an entry of its own that carries the signed handover, written with its first act, and acts as owner under that entry. The acceptance keeps the handover standing when the old owner forks below it. When a forked owner handed over to two devices and both accepted, neither counts. | #RUST-39 |
| 2026-11-14 | A member leaves with an operation of its own; the owner cannot, it hands over first. A leave beats every admission, promotion and handover of the leaver that it had not seen; a handover that is void this way leaves the old owner the owner. Against a removal that had not seen it, both stand and the removal's cut holds. | #RUST-39 |
| 2026-11-14 | A leave carries no new key, because the leaver would know it. The key changes the way it does after two removals at once: the old one is still held by a device that is out, so it is nobody's current key and the next writer makes one. The same after a fork. | #RUST-39 |
| 2026-11-14 | When the owner forks, the team has no owner from there on (built so; whether an admin should succeed is asked, see open questions). Members and admins go on; nobody makes or removes an admin. | #RUST-39 |
| 2026-11-14 | The owner's several devices: one device acts as owner at a time. The role sits on one named device and moves between the person's own devices with the handover that exists; the person's other devices are admins meanwhile. Answered on the card of the #RUST-39 session, recorded 2026-11-17. | #RUST-17 |
| 2026-11-14 | A device's revocation is logged in each team's log: a team operation with a cut, like a removal, written by another device of the same user or by anyone who outranks the device. One entry per team; a team the revoking device cannot reach hears of it late. Not built yet. | #RUST-17 |
| 2026-11-14 | When the owner's device forks, the longest-standing admin may take the team over with an entry of its own. With no admin at that moment the team has no owner. Built 2026-11-17, see the rows of #RUST-43. | #RUST-17, #RUST-39 |
| 2026-11-17 | The user is one 32-byte seed. The 24 words are BIP39 with the English list over the seed as the entropy; BIP39's stretching of the words into a wallet seed is not used. Written in `identity` without a BIP39 crate: forty lines and the list, which is copied unchanged from the BIP (SHA-256 `2f5eed53…dbda`). | #RUST-17 |
| 2026-11-17 | The user key is Ed25519 with the secret SHA-256("rerusted/user/1" ‖ seed), the same construction as a device's signing key under another context string. It signs the certificate of the first device, or of one set up from the words, and no log entry. | #RUST-17 |
| 2026-11-17 | The user's seed lives on no device: devices vouch for each other. A new device is added by scanning a QR on a device the user has, which signs its certificate. The seed is used on the first device and for recovery. Devices that a revoked device vouched for stay, unless revoked too. Answered by Danny on the plan card of the #RUST-17 session. | #RUST-17 |
| 2026-11-17 | Device certificates never run out; only a revocation removes a device. No validity window and no clock, so replicas keep resolving the member list from the log alone. A lost device stays trusted until it is revoked. Answered on the same card. | #RUST-17 |
| 2026-11-17 | Device certificate, version 1: 161 bytes, hand-written like an entry: version byte first and signed, user, device, signer, then the signer's Ed25519 signature over a context string and the body. The signer is the user key or a device of the user's. A device proves itself with a chain: one count byte and up to 16 certificates, the user key's first, each later one signed by the device before it, no device twice. | #RUST-17, #RUST-29 |
| 2026-11-17 | Test vectors are text files in `crates/identity/tests/vectors/`, one per format and version, computed outside the crate where that is possible (Python). | #RUST-17 |
| 2026-11-17 | "Longest-standing admin" is read from the log: of the devices that the owners' operations leave as admin, the one whose present unbroken stay as admin began first in the resolved order of operations, the lower device id on a tie. A demotion or a leave ends a stay; a former owner counts from when it first became admin or owner. | #RUST-43 |
| 2026-11-17 | The take-over is entry kind 9, `claim`: team, under, parents and the forked owner it is about. It counts only from the heir, written as the admin it is; the first such claim in its log makes it owner from that position. It is not in the `teamlog` trait: `Replica::take_over` and `Team::heir`. | #RUST-43 |
| 2026-11-17 | An accepted handover of the forked owner stands against a claim. An admin that forked is passed over as heir unless it claimed before its fork. A forked owner is never its own heir. When a claim turns out void (the handover turns up, or the fork is proved lower and the heir differs), what was done under it is void too. | #RUST-43 |
| 2026-11-17 | A new device of a person gets into their teams through their own devices: a device of the same user that is in the team admits it, at most at its own rank, the owner's device as admin. No admin of the team has to act. A broken-into device can do the same for a thief until it is revoked. Answered by Danny on the plan card for #RUST-42. | #RUST-42 |
| 2026-11-17 | When a person revokes their owner device from another of their devices, the device that revoked it is the owner from the revocation on. Whoever of the person and a thief revokes first wins. Answered on the same card. | #RUST-42 |
| 2026-11-17 | The team's rules stay per device. A device publishes its certificate chain as entry kind 10, `certify`; the first one counts and an entry with a chain that does not lead to its author is refused. Admission and revocation among one user's devices are kinds 11 and 12, laid out like `set` and `remove`. They count like an owner's operations; a removal between two devices of one user is read as a revocation. Not in the `teamlog` trait: `Replica::certify`, `admit`, `revoke`, `user`. | #RUST-42 |
| 2026-11-17 | Two devices of one user that revoke each other unseen are both out, and the team has no owner if one of them was. Chosen over a winner by lowest hash, which a thief could grind for. A revoked device that has heard of it cannot revoke back, but it can pretend it has not and take the revoker out too. | #RUST-42 |
| 2026-11-17 | Whether an admission or revocation among one user's devices counts is settled in rounds: all are taken to count, the team is resolved, those whose author is then not in the team where they stand are dropped, until nothing changes; a circle counts as a whole. | #RUST-42 |
| 2026-11-17 | Debug builds optimise `identity`, `curve25519-dalek`, `ed25519-dalek` and `sha2` (profile overrides in the workspace manifest): the random test took 173 s without and 1.4 s with them for 40 runs. Its default is now 400 runs. | #RUST-42 |
| 2026-11-17 | An invite's secret is the seed of an Ed25519 key pair. The log holds the public key and the expiry (entry kind 13); the invited signs a request for one device with it; the admission (kind 14) carries that signature. So every replica can check an admission against its invite without knowing the secret, and no MAC or new dependency is needed. | #RUST-44 |
| 2026-11-17 | Only the inviter's device writes the admission for its invite, and the invite has to stand earlier in the same log. One log has one order, so "works once" needs no rule for two admins using one invite at the same time. Cost: the inviter's device has to be reachable; that is why the invite names it. | #RUST-44 |
| 2026-11-17 | The expiry is checked once, by the inviter's device, against the time its caller passes. The log's rules stay free of time. An admission with an invite that does not hold (unknown, used, for another device) is an entry that changes nothing. | #RUST-44 |
| 2026-11-17 | An invite lets a device in as a member. Other roles are given afterwards by whoever may. The text form of an invite (link, QR) is the caller's. | #RUST-44 |
| 2026-11-17 | The team log is our own signed log in `identity`; p2panda is not used. #RUST-17 goes ahead as written, on the design of the #RUST-39 spike. | #RUST-3, #RUST-17 |

## Open questions

| Date | Question | Source |
|---|---|---|
| 2026-11-10/11 | CI runner: write it ourselves, or build on an existing Rust runner (chimera) and write only the planner? | #RUST-9, #RUST-18 |
| 2026-11-10/11 | Base system for the image. | #RUST-18 (#RUST-12) |
| 2026-11-10/11 | Label granularity: per provider (`twitter`) or per account and kind? | #RUST-2 |
| 2026-11-10/11 | Is `allow` per wire enough, or does it need an expiry? | #RUST-2 |
| 2026-11-10/11 | Dynamic children (one subtree per list item): template in the `view` primitive only, or a `for-each` in compose? | #RUST-2 |
| 2026-11-17 | After two devices of one user revoked each other unseen and one was the owner, the team has no owner and nobody can take it over: the heir rule applies after a fork only. Should the longest-standing admin be the heir here too? | #RUST-42, #RUST-43 |
| 2026-11-17 | The owner's device forks and the same person has another device in the team. Today the longest-standing admin is the heir, who may be somebody else. Should the person's own device come first? | #RUST-42, #RUST-43 |
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

Answered: "p2panda behind our own trait, or our own signed log?" (#RUST-3) was answered on 2026-11-14: our own log, built in `identity` (#RUST-39), so #RUST-17 goes ahead as written. "Painter: own on Vello, zgui, or gpui?" (#RUST-8) was answered on 2026-11-13: gpui. "Licence?" (#RUST-6) was answered on 2026-11-13: Apache-2.0. "Light theme now or later?" (#RUST-5) was answered on 2026-11-11 in #RUST-15: dark only for now. "Concurrent admin actions need one fixed rule" (#RUST-3) was answered on 2026-11-14: only the owner removes or demotes an admin. The three questions the first spike left open (a fork of a device's log, handing the team over, leaving) were answered the same day, see the decisions of #RUST-39.

## Contradictions between issues

Listed, not resolved here; an item says so when it was resolved since.

1. **Teams.** #V-1 (2026-11-04): "we dont want the concept of a team in there". #RUST-3 and #RUST-18 (2026-11-10/11) build on teams: the member log, the team key, invites and the `team` scope.
2. **Server.** #V-1 (2026-11-04): "exponential will get multiplayer and also gets a server". #RUST-18 (2026-11-10/11): "Everything p2p; no server holds state or authority."
3. **Agents.** #RUST-18 (2026-11-10/11), principle 6: "Only sessions: no separate agent concept." #RUST-2 and #RUST-4 (2026-11-10/11) have an `agent` primitive, and #V-1 (2026-11-04) lists an eAgent library.
4. **`code` means three things.** The `code` module in #RUST-18, the `code` primitive (a WASM component) in #RUST-4 and #RUST-2, and the `code` interface (repos) in #RUST-4 and #RUST-9 (all 2026-11-10/11).
5. **Missing reference.** #RUST-9 (2026-11-10/11) defines the Forge events as "the list in #NEW-36", an issue that is not on this board.
6. **Licence.** Resolved 2026-11-13: Apache-2.0 (#RUST-6). #RUST-6 (2026-11-10/11) had assumed it "as for the earlier desktop work" while #RUST-18 listed it as open decision 4.
