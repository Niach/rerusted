# Lessons from eDE: computer use and Voice

Status: second part, as of 2026-11-20 (eDE master `30abe355fd`, eDE 0.2.30). The first part (L-01 to L-61) was written on 2026-11-17 19:33 UTC; the second updates their status and appends L-62 to L-85: the agent spaces core as built, the real runs, the tests and the call seal.
Source: #EDE-118, from the review #EDE-69

What the team learned building computer use and Voice in eDE (Exponential Desktop, repository Niach/exponential), so Rerusted has them right from the start. Each rule gives the mistake eDE made, why it happened, the design rule that prevents it, and its sources. This is not a spec: `docs/spec/06-*.md` (voice) and `docs/spec/07-*.md` (compositor) stay reserved, and a design is checked against these rules when they are written.

## How to read it

- The review #EDE-69 ran on 2026-11-17: 28 review areas, 292 findings, 84 raw lessons. They say L-01 to L-61. No adversarial verification ran on the findings; the coordinator read 19 of them against the code (#EDE-69).
- Issues (#EDE-nn, #EXP-nn) are issues of the Exponential tracker. Pull requests are written `Niach/exponential#nnnn`. A rule whose findings have no issue yet names the review, #EDE-69.
- "eDE" under a rule is the status in Niach/exponential on master `30abe355fd` (2026-11-20); for L-01 to L-61 it was first written when the review state was archived on 2026-11-17, and the second part updated it where eDE fixed or decided something since: **fixed** (the PR is on master), **in work** (an issue, a PR open or none yet), **filed** (an issue only), **not filed**, or **needs a redesign**.
- Four rules are decisions Danny took on 2026-11-17, marked so. They are decisions for eDE. They are not rows in [decisions.md](../decisions.md): that file holds what was decided for Rerusted.
- Background computer use: on 2026-11-17 only the preparation landed; see [Spaces](#spaces-and-why-background-mode-did-not-fit). By 2026-11-20 the core (#EDE-125, #EDE-126) is on master and proved in a VM, behind a switch; offering it to people (step F5) still waits. See [The agent spaces core as built](#the-agent-spaces-core-as-built).
- Sources of the second part are attachments: `core-design.md` and `core-design-verdicts.md` (the agent spaces core, 2026-11-17 21:43), `space-proof.md` (2026-11-17), `space-proof-master.md` (2026-11-19), `release-check.md` (last upload 2026-11-18) and `release-check-0.2.29.md` (2026-11-19) on #EDE-69; `call-seal-design.md` and `call-seal-verdicts.md` (last upload 2026-11-18 06:08) on #EXP-1941. `core-design.md:47` is a line of that attachment; a code path with a line number is on eDE's master.

## Input and who owns pressed state

### L-01 Every injector is an input source the compositor owns

- Mistake: agent input went into the person's seat as loose press and release events over throwaway connections. One pressed set, one modifier state; a client that died left its keys down, and a release could be refused (lock, protected surface).
- Why: input was a thin IPC layer over the functions written for the physical keyboard, and the backend mirrored XTest, one stateless call per event.
- Rule: pressed state belongs to the source (a connection, a device); the compositor releases it in one place when the source ends, is refused or loses the seat, and it never refuses a release.
- Test: two connections press the same key, one disconnects: the other still holds it, and at the end nothing is down.
- eDE: fixed. #EDE-73 (Niach/exponential#2939), #EDE-80 (Niach/exponential#2947); checked on a real session (C1 in vm-run.md on #EDE-69). Left: without a hold, person and agent still share one key state until the Desk split (#EDE-90).

### L-02 Client code never pairs press and release itself

- Mistake: drag, click and key combos were straight-line code with `?` between press and release, so a failed step left a button or modifier down.
- Why: backend calls return a result, the tools chain them, and the fake backend could never fail.
- Rule: every pressed key or button is a guard that releases on every exit path, and the report says what could not be put back.
- Test: fail each step in turn; after each, nothing is down.
- eDE: fixed. #EXP-1895 (Niach/exponential#2937).

### L-03 Text and chords are two operations

- Mistake: one `Key::Char` served both text and shortcuts and was resolved by a different rule per OS, so non-US and multi-layout keyboards typed the wrong keys.
- Why: the vocabulary came from xdotool on X11, each port guessed an equivalent, and nobody tested another layout.
- Rule: text is layout-free; a chord is modifiers plus a key resolved on the active layout to a physical key.
- Test: one fixture of layout and combo to expected key, replayed by every backend.
- eDE: fixed. #EXP-1907 (Niach/exponential#2963), #EDE-84 (Niach/exponential#2961), #EXP-1922 (Niach/exponential#2980). Filed: #EDE-85 (a character no configured layout makes).

### L-04 Taking the seat is one state machine, shared by every backend

- Mistake: the X11 hold grabbed the devices that existed at that moment: keys held at take stayed stuck, a device that reconnected slipped out. Each OS backend owned the whole lock lifecycle and each fix landed in one copy.
- Why: the backend trait hid everything above the OS primitive, so every port rewrote the lifecycle from the previous one.
- Rule: a backend exposes only the OS primitive (install or remove a filter, classify one event); one shared state machine owns take, "let the releases of what was down through", Esc once and the teardown order.
- Test: replay every edge against a fake seat, once for all platforms.
- eDE: the defects are fixed, #EXP-1906 (Niach/exponential#2956), #EXP-1921 (Niach/exponential#2977). The shared state machine is not filed (#EDE-69).

## Focus and what may take the keyboard

### L-05 Only windows the person started take the keyboard

- Decided by Danny, 2026-11-17, for eDE.
- Mistake: every newly mapped window was focused, whoever opened it. An updater prompt or a window an agent launched took the next keystrokes, Enter included.
- Why: the code grew from a single-seat example where "last caller wins" looked fine.
- Rule: a new window takes the keyboard only when the person's own action led to it (their launch, their focused app, a valid activation token); every other window opens behind, unfocused.
- eDE: fixed. #EDE-76 (Niach/exponential#2954), #EDE-82 (Niach/exponential#2950); F3 passes on a real session (vm-run.md, recheck.md on #EDE-69).

### L-06 Keyboard focus is one state machine per seat, with a stack

- Mistake: the promise that a sealed prompt has the keyboard lived in the client; the compositor focused layer surfaces case by case and did nothing when one closed. The start menu had moved to a layer with no keyboard on 2026-11-08 and nobody saw it until a person typed on a real session.
- Why: focus policy grew one feature at a time with no owner, and the tests played the lock only, never a prompt or a menu.
- Rule: a surface that claims the keyboard pushes on map and pops on unmap, in the compositor, and the surface below gets the keyboard back.
- Test: per surface kind, headless: open it, type, the keys arrive there; close it, type, the keys arrive in the window below.
- eDE: fixed. #EDE-71 (Niach/exponential#2932), #EDE-86 (Niach/exponential#2974). #EDE-107 is decided and fixed: a sealed prompt is drawn above a fullscreen window and takes the keyboard and the click (Niach/exponential#3063).

### L-07 A focus request names its seat and its origin

- Mistake: `focus()` could not tell a new window, a client request, the shell, an agent and the person's click apart, and its target type could not express X11. The first X11 window of a session read `focused: true` and got no key until it was clicked.
- Why: single-seat use needed no caller; X11 was added beside the Wayland path.
- Rule: every focus, raise and placement request carries a seat and an origin, and the focus target is one enum over Wayland, X11, layer and lock surfaces, so the protocol handoff exists once.
- eDE: fixed for X11, #EDE-88 (Niach/exponential#2981). The Desk landed: focus, the drive paths, the clipboard, placement, the hold and the lock checks take the desk they act on (#EDE-90, Niach/exponential#3008). An origin on every request is not filed (#EDE-69).

### L-08 What is drawn, what is hit and where the pointer is come from one list

- Mistake: drawing order, hit-testing and pointer focus were three hand-written walks that disagreed: a press on a title-bar button reached the window under it, and a hidden panel took clicks under a fullscreen window. Window lifecycle lived in four containers with copied fix-ups.
- Why: frames, popups and "fullscreen draws alone" were added to the renderer only.
- Rule: one ordered list of regions per output and one window registry with an explicit state; rendering, hit-testing, captures and the pointer refresh derive from them.
- eDE: fixed for the hit-testing defects, #EDE-103 (Niach/exponential#2994). Two more walks disagreed and were found only on a real session: a press on a toast or the start menu closed the X11 window whose title bar lay beneath (the title-bar test ran before the layers, #EDE-119, Niach/exponential#3018), and a press on a popup of a panel layer focused the window under it (#EDE-123, Niach/exponential#3021). The single registry is not filed (#EDE-69).

### L-09 System chrome is one overlay primitive that takes no focus and no click

- Mistake: the computer-use banner, the call pill and the push-to-talk capsule were pop-up windows assembled per caller; on Wayland each was a normal window that took focus. The indicator lived in whichever process hosted the run.
- Why: it was built for an IDE on X11, and no test opened these windows on a Wayland compositor.
- Rule: one overlay surface with a stated contract (no keyboard, no pointer, top layer, never on a fullscreen display), drawn by the component that owns the hold; where the contract cannot be kept, it does not open.
- eDE: fixed. #EXP-1912 (Niach/exponential#2964, Niach/exponential#2966, Niach/exponential#2969), #EXP-1923 (Niach/exponential#2982). The pill was pictured from a specimen, and moving the indicator into the compositor is not filed (#EDE-69). A real call did run since: the sealed-call proof (`call-seal-proof.md` on #EXP-1941, 2026-11-19) has the desktop caller's pill on screen.

## The hold and the person's abort

### L-10 Only input tools take the hold

- Decided by Danny, 2026-11-17, for eDE.
- Mistake: every tool call engaged the hold. `screenshot`, `list_windows` and `wait` took the keyboard and pointer of a person in a fullscreen game.
- Why: the hold was a side effect of "any call"; the gate that decides whether a call may act was put in front of it later, in another layer.
- Rule: observation never takes input; only a tool that injects input acquires the hold.
- eDE: fixed. #EXP-1898 (Niach/exponential#2945).

### L-11 The hold is a lease owned by the work in flight

- Mistake: the hold's lifetime was inferred from a timestamp stamped at call start. A `type` call of 70 s lost the hold while it was still typing, and parallel calls touched the seat from any pool thread.
- Why: it was designed for one run, one lock and short actions; long calls, several runs and parallel calls came later and reused the flags.
- Rule: one actor per seat runs actions one at a time; the hold lasts as long as an action is in flight plus an idle time that starts when the queue is empty.
- eDE: fixed. #EXP-1893 (Niach/exponential#2934).

### L-12 One cancel token per run, fired by every ending

- Mistake: only Esc reached a running call. A session end, a Stop or the switch going off flipped a map entry and the call went on typing.
- Why: each ending was wired where it was first needed, and a tool call was never a tracked object.
- Rule: a tool call is a job owned by its run; Esc, Stop, session end, switch off, idle and host shutdown fire the same token, every step checks it, and a run's jobs are joined before anything shared is released.
- eDE: fixed. #EXP-1894 (Niach/exponential#2938).

### L-13 Only the person's own message ends an abort

- Mistake: after Esc, "a turn started" stood for "the person spoke". Bus posts and child wake-ups start turns too, so the hold was taken again unasked.
- Why: computer use hung on engine hooks that existed for other reasons and changed meaning when the session tree arrived.
- Rule: an abort is cleared only by an event typed as input authored by the person.
- eDE: fixed. #EXP-1896 (Niach/exponential#2936).

### L-14 "Held" is derived from events, not from a handle

- Mistake: the X11 lock could die on its own thread (the connection broke) while the host kept the handle as proof that the lock was held.
- Why: the lock's only channel back was the Esc callback.
- Rule: whatever takes the person's input reports every way it can end (released, aborted, lost) on one channel, and a lost hold is an abort.
- eDE: fixed. #EXP-1906 (Niach/exponential#2956).

## The clipboard

### L-15 A clipboard that cannot be saved whole is never touched

- Decided by Danny, 2026-11-17, for eDE.
- Mistake: paste copied the person's clipboard out, overwrote it, wrote it back, and reported "as it was" whatever was lost to size caps, timeouts or lazy formats.
- Why: borrowing is the only portable trick on X11, macOS and Windows, and the lossy cases were treated as detail in each backend.
- Rule: an incomplete save is a refusal, never a best effort, and a result says "as it was" only when that was verified.
- eDE: fixed. #EXP-1900 (Niach/exponential#2953). The backends are strict and may refuse often (#EDE-69).

### L-16 An agent has its own selection

- Mistake: the borrow was carried over to the one platform where the compositor is ours; two runs shared it, and the person's content depended on the agent host staying alive.
- Why: there was one seat and one selection, so paste could only mean swapping it.
- Rule: the agent's seat has its own selection, or text is committed directly; the person's selection is never read or replaced.
- eDE: fixed for agent spaces: a clipboard per desk (#EDE-126 step 12, Niach/exponential#3075), and a space has its own clipboard with no hold and no gate (step 18, Niach/exponential#3084). Foreground computer use on the person's desk still borrows: narrowed by #EXP-1920 (Niach/exponential#2970, Niach/exponential#2971), a killed host still loses the restored X11 clipboard (#EDE-69).

## Grants, identity and the device switch

### L-17 The device switch is the gate; a session cannot turn computer use on for itself

- Decided by Danny, 2026-11-17, for eDE.
- Mistake: the capability that takes the person's seat was a plain session field. Anything able to post to the session could set it, the agent included, and the machine read it as an override of its owner's switch.
- Why: the per-run pick was designed as a person's toggle and then reused through generic paths without asking who consents.
- Rule: effective capability is the minimum of the device owner's grant and the session's request, enforced on the machine that owns the seat; the session can only lower it, and switching it off ends it for running sessions.
- eDE: fixed. #EXP-1899 (Niach/exponential#2990) with its parts #EXP-1903 (Niach/exponential#2951), #EXP-1904 (Niach/exponential#2962) and #EXP-1928 (Niach/exponential#2988); #EXP-1918 (Niach/exponential#2967). A session started over MCP takes its starter's value; that reading is the coordinator's, not Danny's (#EDE-69).

### L-18 A default is resolved once, where the row is created

- Mistake: "absent means the device's default" lived in a start frame. When the start became a row with a non-null boolean, the third state vanished: three clients showed the toggle on, omitted the key, and the run got `false`.
- Why: each layer assumed another one would resolve it, and no fixture case had an absent key.
- Rule: the server resolves the default when it creates the row and stores the result; clients send what they show; control and status are separate fields.
- Test: a fixture case with the key absent, replayed by the server and all four clients.
- eDE: fixed. #EXP-1903 (Niach/exponential#2951), #EXP-1928 (Niach/exponential#2988), both parts of #EXP-1899.

### L-19 The run's identity reaches the compositor

- Mistake: the run was dropped right after the bearer check. Below it there was one process-wide grant (session `epilot`, capture and person input, no expiry, never revoked), one lock, one banner.
- Why: "the desktop" was one thing, so nothing below the server needed to know who was calling.
- Rule: run, target and grant travel end to end; a grant is minted per run with only its target's capabilities and an expiry, and it is revoked when the run ends.
- eDE: fixed as preparation. #EXP-1897 (Niach/exponential#2940), #EXP-1924 (Niach/exponential#2976), #EXP-1925 (Niach/exponential#2979). The per-run grant ran against a fake socket only (#EDE-69).

### L-20 The session is discovered from the OS at call time

- Mistake: a user unit started at boot had no session in its environment. The daemon said "No display session"; the polkit agent registered for its own process instead of the login session, logged "registered", and no prompt ever appeared.
- Why: it worked from an IDE started inside the session, where the environment is right by accident.
- Rule: never read the session from a long-lived process's environment; ask the OS when it is needed, and log what was registered for.
- eDE: fixed. #EDE-87 (Niach/exponential#2975), #EXP-1918 (Niach/exponential#2967); the daemon half has not run against a live unit (#EDE-69).

## Run lifecycle and teardown

### L-21 A run owns what it acquired; cleanup never goes by session id

- Mistake: grants, the hold, the abort hook and scope units were keyed by the session id and released by one long end sequence. Two runs of one session can overlap, so a late end hit the newer run, and a panic or failed spawn skipped the list.
- Why: a session used to be exactly one process; respawn, suspend and revive came later.
- Rule: every run has a generation and one handle whose drop releases exactly what that run acquired; on any end, take power away first (revoke, cancel, release) and report afterwards.
- eDE: partly fixed, #EXP-1910 (Niach/exponential#2960). Filed and still in the backlog on 2026-11-20: #EXP-1917 (run-scoped cleanup).

### L-22 One authority says a session ended

- Mistake: the device decided from local outcome strings that a session ended and destroyed volumes and simulators, while the server kept root chats and lost runs alive. The cleanup lived in one of the two host types.
- Why: the end rule was copied to the device when ends were simple and never followed the server's exceptions.
- Rule: the device destroys only on the synced row's verdict, in one idempotent sweep that every host type runs and that checks liveness before each step.
- eDE: fixed. #EXP-1909 (Niach/exponential#2957): the device follows the synced row. #EXP-1911 (Niach/exponential#2959) is the liveness half: the daemon's sweep of ended sessions stopped the scope units of a session that still had a live run on the host, and now checks the host-wide run registry first.

### L-23 One runner per desktop owns runs; UIs are its clients

- Mistake: a local session's run state belonged to whichever process won a file lock. The store row was a mirror nobody reconciled, so a stopped or crashed run stayed `busy`.
- Why: the shell and the app each embed the runner; the lock was added to stop double runs, not to define an owner.
- Rule: one service owns runs, tools and change events, and persisted run state carries an owner lease (pid and epoch) so a stale one is detected on takeover.
- eDE: the two defects are filed and fixed in #EDE-134 (Niach/exponential#3104): a Local run stopped mid-turn stayed `busy` for ever (`apps/desktop/crates/elocal/src/runs.rs:966`), and when the Exponential app held `runs.lock` the shell's Local chat never showed the reply (`runs.rs:434`). The one owning service with a lease is not filed; this rule has no other source.

### L-24 State that outlives a process uses stable identities

- Mistake: persisted restore data held window ids from a process-local counter, and the shell supervisor took "all factory" as its state at each start.
- Why: each piece was written and tested inside one process run.
- Rule: anything persisted or compared across restarts is keyed by a stable id and derived from the durable store; a subscription delivers the current snapshot with the subscribe.
- eDE: two supervisor defects are fixed, #EDE-110 (Niach/exponential#3002). The rest is not filed (#EDE-69).

## The control plane

### L-25 The control plane never blocks, and a call has one reply path

- Mistake: the control socket did blocking I/O on the compositor's event loop, pixels went through the same stream, and a deferred reply had its own branch with a busy flag set after the handler ran. A slow or dead client stalled the desktop, and a clipboard read that finished early never answered.
- Why: a blocking client on the event loop was the shortest way to a working protocol; deferred replies were added later, for the clipboard only.
- Rule: bounded queues per connection, bulk data (frames, clipboard bytes) as file descriptors, a deadline on every call, and one reply path on which a call is registered as pending before its handler runs.
- eDE: fixed. #EDE-74 (Niach/exponential#2946).

### L-26 A protocol change says what an older peer does

- Mistake: the protocol grew by adding enum variants and matching error text. Since Niach/exponential#2953 the desktop app counts an eDE without the new `unread` field as a partial save, so against an installed older eDE it refuses `paste_text` and `copy_selection` until eDE's next release.
- Why: both sides live in one repository and were always built together.
- Rule: hello negotiates a feature list, unknown values parse leniently, errors are typed codes, and every change states the behaviour against the previous release.
- eDE: the case is known and accepted, #EXP-1900. Done right in #EDE-91 (Niach/exponential#2987): an absent `space` means the person's.

### L-27 Every long-lived socket goes through one connection type

- Mistake: the control socket learned ping and a silence watchdog (#EXP-414); the two call sockets written later repeated the bare read loop without them.
- Why: the transport policy lived in one function's body, so a new socket copied the shape and missed the lessons.
- Rule: one connection type with ping, an inbound-silence deadline and bounded sends; a bare `stream.next()` loop does not pass review.
- eDE: fixed for calls, #EXP-1929 (Niach/exponential#3000). The shared type is not filed (#EDE-69).

### L-28 UI code enqueues messages, and nothing external runs on the UI thread

- Mistake: the shell was one mutable state behind `Rc<RefCell>` with about 300 borrow sites, and views called the host during their own updates. On eDE 0.2.6 a press with a taskbar preview open killed the shell ("RefCell already mutably borrowed"); hiding the push-to-talk capsule panicked by re-entering its own entity. D-Bus without timeouts, process spawns and full-table reads ran on the same thread, so one hung tray app froze the panel.
- Why: each new hook borrowed the state just once, re-entrancy shows only at run time, and blocking calls are quick on a development machine with healthy peers.
- Rule: state has one owner; callbacks enqueue typed messages and a reducer applies them; opening or closing a window is an effect run after the update; every external call runs on a worker with a deadline and returns as an event.
- Test: every surface has a test that opens it, clicks it and closes it.
- eDE: the panics are fixed, #EDE-75 (Niach/exponential#2935), #EDE-89 (Niach/exponential#2984), #EXP-1901 (Niach/exponential#2941); compare.md on #EDE-69. The calls on the UI thread are fixed for the tray and the services: #EDE-135, tray calls on their own thread with a timeout (Niach/exponential#3102), each services domain polled on its own thread with timeouts (Niach/exponential#3098). The single owner is not filed (#EDE-69).

### L-29 Every action a document can fire has a typed handler in every host

- Mistake: the shared chat document fired actions the eDE host only partly implemented: `stop` was an empty arm, a failed send lost the composer's text, a missing argument defaulted to "open".
- Why: documents are JSON, handlers match on strings, and the tests grep for names.
- Rule: the host-function contract is generated from one spec, handlers are exhaustive at compile time, and a fixture replays each document's events against the real host.
- eDE: the defects are filed and fixed in #EDE-134 (Niach/exponential#3104): Stop works, a failed send keeps its text. The generated contract with exhaustive handlers is not filed; it has no source beyond the review (#EDE-69).

## The lock and sealed prompts

### L-30 One function answers "does this session lock, and how"

- Mistake: an empty lock command changed from "no lock" to "the sealed lock", and three of five entry points kept the old meaning. The default configuration did not lock on suspend.
- Why: the decision lived as copies of an `is_empty()` test, and the tests covered parsing and the non-default path.
- Rule: one typed decision per policy, called by every entry point; test-only powers are compiled out of production.
- Test: stock configuration, the external trigger in (suspend, `loginctl lock-session`, the menu's button), a locked frame on every output out.
- eDE: fixed. #EDE-70 (Niach/exponential#2933); L1 to L4 pass on a real session (vm-run.md on #EDE-69).

### L-31 Locked is the compositor's state, not the lock client's

- Mistake: the lock screen did not survive a display change or its client's death cleanly, and one client drew for all outputs.
- Why: CI ran the lock tests without the real lock client (L-55), so these paths were never exercised.
- Rule: while locked, every output is covered whatever happens to the client; a dead client is restarted and the desktop is never shown in between.
- eDE: fixed. #EDE-72 (Niach/exponential#2942), #EDE-77 (Niach/exponential#2944), #EDE-78 (Niach/exponential#2948); L5 and L7 pass (vm-run.md). Left: a mode change loses the text typed so far (#EDE-69).

### L-32 A credential travels over an inherited descriptor and is checked by peer identity

- Mistake: the lock passed its unlock secret through the environment, and the person-input path was guarded by a token file every process of the user can read.
- Why: the environment was the easy channel, and nothing states who may read a credential.
- Rule: never a value any process of the user can read; the compositor binds the principal to the grant itself.
- eDE: fixed for the lock, #EDE-72 (Niach/exponential#2942), #EDE-79 (Niach/exponential#2949). The token file did not change: the compositor still writes the root grant's token to a file in the runtime directory, mode 0600, readable by every process of the user (`apps/ede/crates/ede-compositor/src/ipc.rs:179-183`). The core's verdict D14 shows that a launch into an agent space could reach it (`core-design-verdicts.md:245-249`). Not filed; see L-70.

### L-33 A prompt works when the right password went through the real stack

- Mistake: on Ubuntu 26.04 with polkit 127 the polkit prompt had three defects at once: no prompt (L-20), the right password always refused (the agent started a helper that is no longer setuid; polkit 127 uses a socket), and the agent exited after one Cancel with nothing to restart it. All three were in the released 0.2.6.
- Why: nothing ran it in a real session on that distro before the VM run.
- Rule: a prompt is tested per supported distro in a real session: wrong password, right password, Cancel, lock while it is up, and once more afterwards.
- eDE: fixed. #EDE-87 (Niach/exponential#2975); 9 of 9 pass (polkit-check.md on #EDE-69). Open: Esc and Ctrl+C do not cancel the prompt (#EDE-69).

## Capture and screen memory

### L-34 A frame carries its capture time and its geometry

- Mistake: privacy was enforced by matching pixels to a window list sampled at another moment, and each capturer decided what the settle interval means. After a resume the first frame was stale.
- Why: three OS capture APIs sit behind one trait, and the window list and the pixels never come from one snapshot.
- Rule: the engine alone enforces settle and masking from the frame's own time and geometry; where the compositor is ours, protected and denied windows are masked in its own render.
- eDE: fixed for the defects, #EXP-1908 (Niach/exponential#2958). A sealed prompt was masked only on the fullscreen path and drawn in every other capture; now it is black in every capture (#EDE-121, Niach/exponential#3066). Filed, still in the backlog: #EXP-1913.

### L-35 The tested backend and the real backend share the pipeline

- Mistake: headless had an unconditional frame clock, no damage and no flip queue, and captures used a second renderer that could not read what the screen renderer shows. On real hardware the windows of GPU clients were missing from captures.
- Why: the real backend grew from an example as one file; headless was written for pixel tests.
- Rule: one per-output frame scheduler (events in, actions out) that every backend drives, and capture through the renderer that draws the screen; state is updated at the event it is named after (presented means the flip was acknowledged).
- eDE: fixed. #EDE-100 (Niach/exponential#2998), #EDE-102 (Niach/exponential#2993), #EDE-109 (Niach/exponential#3003). #EDE-113 is fixed too, see L-78. Not yet seen on a real GPU (#EDE-69).

### L-36 Capturer health is a state the person sees

- Mistake: a failed grab was a debug log, the status knew only off, active and paused, and eDE became a capture platform through an unrelated probe, without a spec or a test.
- Why: the capturer trait returns frames or strings; nothing models "the session ended".
- Rule: health is a state (capturing, paused, failed with a reason, reopening), and a platform path nobody declared stays off.
- eDE: not filed, and no issue or PR since gives it a source; it stands on the review alone (#EDE-69). The nearest open issue is #EXP-1913 (a docked display records nothing until a restart, and nothing says so).

## Displays and their identity

### L-37 A display has a stable id, and a coordinate belongs to its frame

- Mistake: displays were 1-based position numbers, kept in step over three caches by an epoch counter; a display's id changed when its twin appeared. Logical and physical pixels were told apart on macOS only, and coordinates carried no frame identity, so a click computed from an old screenshot could land on a changed layout.
- Why: the numbering was inherited from the tools' display index, and geometry was read once at session start.
- Rule: every stored reference is a key that does not depend on the other displays; geometry (position, pixel size, scale) comes from the same event as the pixels; a call whose frame (display id plus epoch) is no longer current is refused, never re-mapped.
- eDE: fixed for the ids and the pixels, #EXP-1919 (Niach/exponential#2968), #EXP-1897 (Niach/exponential#2940); #EXP-1908 (Niach/exponential#2958) refuses a stream frame of the wrong size. Left: two same-size displays that swap places, and coordinates without a frame (#EDE-69).

## Spaces, and why background mode did not fit

eDE's compositor has one seat. The plan (background-mode-plan.md on #EDE-69) lists nine causes that block an agent working beside the person:

| # | Cause in eDE | Rule |
|---|---|---|
| 1 | One `wl_seat`; every drive path uses it | L-38 |
| 2 | One keyboard focus, one window stack, one activation | L-38, L-07 |
| 3 | One clipboard; paste swaps the person's | L-16 |
| 4 | One capability for input, hold and clipboard; no op carries a seat or space; windows and outputs have no owner | L-19, L-40 |
| 5 | The hold is the only isolation: it mutes the person's whole input | L-39 |
| 6 | One lock and one cursor: a locked screen stops every drive, capture and render | L-39 |
| 7 | The tool backend has no target ("everything here is foreground") | L-38 |
| 8 | One grant per process; placement is one boolean | L-19 |
| 9 | Shell, screen memory and Voice take every window, focus and pointer as the person's | L-40 |

Status on 2026-11-17: only the preparation landed. A target per run (#EXP-1924, Niach/exponential#2976), a grant per run (#EXP-1925, Niach/exponential#2979), the protocol's space (#EDE-91, Niach/exponential#2987), and the Desk (#EDE-90) with its steps #EDE-111 (Niach/exponential#3005) and #EDE-112 (Niach/exponential#3006); the last step, Niach/exponential#3008, was in the merge queue. There is still one Desk. The core (a second Desk, its socket, the tools without a hold) is being built.

Status on 2026-11-20: #3008 landed, and the core is on master in two tracks, #EDE-125 (the compositor) and #EDE-126 (the protocol, the guards, ePilot), proved in a VM (`space-proof-master.md`). Causes 1 to 8 have an answer for a space (C2 of `space-proof.md` locked the screen while the agent worked); the person's own desk still has one seat, the hold and the borrowed clipboard for foreground use. What the build taught is L-62 to L-72.

### L-38 Model the target from day one

- Mistake: one seat, one focus, one pick, one banner, one lock, one clipboard, as singletons and process statics. Whether a fullscreen window was the agent's or the person's was guessed.
- Why: foreground-only was the first product, so singletons were the cheapest shape and the backend got no notion of who acts where.
- Rule: a target is a seat with its outputs, focus, selection and cursor; every backend call, gate and indicator takes one, and the person's seat is one such target.
- eDE: built. The Desk (#EDE-90, Niach/exponential#3008), then a second Desk with its own seat, output and socket (#EDE-125 step 4, Niach/exponential#3055), and ePilot's `Target::Space` with a space grant per run (#EDE-126 step 16, Niach/exponential#3057). Rerusted has the type (`Desk`, #RUST-23).

### L-39 The hold is not isolation

- Mistake: the only thing between the agent and the person was the hold, which mutes the person. So "the person types while the agent works" is ruled out by construction, and a locked screen stops the agent too.
- Why: the hold answered the foreground case and was then the only tool at hand.
- Rule: an agent space takes no hold and no Esc; the lock and the hold touch the person's desk only.
- Test: the acceptance test of the plan, section (d): one client on the person's socket, one on the space's; 200 person keys interleaved with 200 agent keys, pointer moves, a click and a clipboard set in the space. The person's client got exactly its 200 keys in order, the space's exactly the agent's; the person's focus, pointer and selection are unchanged; the hold was never on; a capture of the space shows the space's client and one of the person's output does not; locking leaves the space running.
- eDE: built and proved. The acceptance test landed early and ignored (#EDE-126 step 6, Niach/exponential#3081); no hold, no gate in a space (step 18, Niach/exponential#3084). In a VM the person typed 400 characters while the agent worked, never paused by more than 0.28 s, with focus right in 66 of 66 samples and the clipboard untouched (`space-proof.md`, C1 to C3), and again on master (`space-proof-master.md`, 85 of 85 and 95 of 95 samples).

### L-40 Every window and output has an owner, and readers say whose they want

- Mistake: the shell, screen memory, the portal pickers and Voice took every window as the person's.
- Why: there was nobody else.
- Rule: window and output records carry their desk; a reader asks for the person's explicitly, and a capability checks the space.
- eDE: fixed in the protocol, #EDE-91 (Niach/exponential#2987). In the core: nothing a space client asks moves the person's desk (#EDE-126 step 8, Niach/exponential#3070), and a space grant sees and drives only its space (#EDE-125 step 14, Niach/exponential#3086).

### L-41 A client's view of seats and outputs is filtered by the library, not by tricks

- Mistake: the closed draft Niach/exponential#1536 added spaces beside the single-seat compositor: a second struct in a new file of 654 lines, a global's space parsed from the library's debug text, hand-written dispatch in place of the stock delegates, a new renderer per capture, no XWayland and a shared session bus.
- Why: the compositor had no place for a second seat, and the library had no filter for the seat and output globals.
- Rule: fix the library (a filter commit, #RUST-33) and keep one type for the person's desk and a space; each space has its own XWayland and session bus.
- eDE: done the way the rule says, see L-62. Rerusted has it (#RUST-23); the filter is required because gpui binds the last seat advertised (#RUST-36). Correction to the first part: eDE did not pin Smithay 0.8. It was on Smithay master rev `65aa86f` (Niach/exponential#2412), and the filter went onto a fork of that rev (L-62). Rerusted's filter commit is on 0.7.0; the spike has no DRM, no dmabuf and no wire protocol.

## Voice: the speech worker, calls, listening, the relay

### L-42 A media path has one clock, and the sender keeps it

- Mistake: the host sent reply audio as fast as it was synthesised, the relay budgeted 60 frames a second and skipped under pressure, Android kept 240 ms and the web queued without bound.
- Why: each side was built in its own PR against the fixture's constants; the fixture gave a budget but not who paces.
- Rule: the sender paces to real time and the contract says so.
- Test: a source at five times real time through the relay into every receiver, no frame dropped.
- eDE: fixed. #EXP-1929 (Niach/exponential#3000).

### L-43 Every long-lived link has a heartbeat with a deadline at both ends

- Mistake: liveness was left to TCP and to a relay idle timer that one live side kept resetting, so a call whose other side had died went on.
- Why: the idle rule was written for "both silent".
- Rule: a two-way heartbeat in the wire contract; a call is one object with a watchdog that fails it.
- Test: a peer that goes silent without closing.
- eDE: fixed. #EXP-1929 (Niach/exponential#3000).

### L-44 The call's orchestration is written once, as a reducer

- Mistake: only the call's phase machine was pure and replayed on four platforms. Timers, the mic check, the queue and the reply watch were written inside UI entities, then copied by hand into the daemon, and the copies drifted within one PR. On the phones, sockets, the audio session and the end paths were written freehand with no fixture, and the shared FFI core fell behind the desktop (#EXP-1857).
- Why: the contract stopped at the pure step, everything with a side effect was "platform code", and the deadline favoured a copy.
- Rule: the core takes time and I/O results as events and returns state and named effects; hosts and phones only execute effects, with one core behind the FFI, and all replay the same fixture.
- eDE: the daemon's defects are fixed, #EXP-1930 (Niach/exponential#3004, Niach/exponential#3009). One host for the IDE and the daemon, and the phones' orchestration, are not filed (#EDE-69).

### L-45 The speech engine publishes one explicit state

- Mistake: `Voices: []` meant no GPU, a retry pending or a failed download. Each host guessed; the daemon turned a transient state into a permanent verdict and advertised "ready" from a probe whose worker had exited.
- Why: events were added per UI need, and the worker protocol assumed a resident worker.
- Rule: one state (starting, downloading, ready, retrying with time and reason, no GPU, failed); a transient state never looks like a verdict, and an advertised capability is proved again when the call is answered.
- eDE: fixed. #EXP-1930 (Niach/exponential#3004), #EXP-1934 (Niach/exponential#3010).

### L-46 Desired state is not a job in the work queue

- Mistake: the voice choice travelled as a job in the same queue as the work; a latest-wins loop dropped it, so a Prepare followed by a Speak lost the voice.
- Why: the loop could not be tested, because it ran only after a real GPU bring-up.
- Rule: voice and language live in their own cell that each work item reads when it starts; protocol logic is never welded to hardware bring-up.
- eDE: fixed, #EXP-1934 (Niach/exponential#3010).

### L-47 A shared cache has one owner, and a fallback decides on proof

- Mistake: the on-disk speech store was guarded by in-process mutexes and cleaned by whichever host thought it owned it; the player fallback relied on which call returned first.
- Why: it was designed for one worker per machine; the daemon became a second host later.
- Rule: one owner, or an OS lock with unique temp names and atomic renames; a fallback decides on bytes consumed or an exit status.
- eDE: fixed, #EXP-1934 (Niach/exponential#3010).

### L-48 Whoever holds a device has a wall-clock deadline

- Mistake: a lost microphone was noticed only when the audio channel closed, and every time limit ticked in received samples. A silent stream never closed and never timed out.
- Why: the first capture was a recorder process that ends with EOF on any failure.
- Rule: device loss is an explicit event, limits run on the clock, one thread owns the listening state, and every request id gets exactly one terminal event (heard, empty, failed, replaced).
- Test: a fake device that stalls, closes and refuses to open.
- eDE: fixed, #EXP-1937 (Niach/exponential#3012, Niach/exponential#3014).

### L-49 One reducer owns the voice session; what a call borrows is a guard

- Mistake: speaking, call phase and push-to-talk arming were written from many call sites and spread by repaint observers. What a call borrowed was given back by an unrelated 30 s heartbeat, and "thinking" ended only when a reply was spoken.
- Why: push to talk, calls and per-agent voices were each bolted onto the speaker entity.
- Rule: every transition is an event from a pure step; microphone, hotkey and voice are guards released when the call ends; busy and queued come from the session's turn state, and every waiting state has an exit for "nothing came".
- eDE: fixed, #EXP-1936 (Niach/exponential#3013, Niach/exponential#3015).

### L-50 The relay forwards only sealed bytes

- Mistake: a call's transcript crosses the relay as readable JSON and its audio as plain Opus, though the relay claims to carry no session content.
- Why: calls were added as a second protocol beside the sealed tunnel; the principle lived in comments.
- Rule: one frame type of opaque sealed bytes plus ids and sizes, so a readable payload cannot be added without changing the type.
- eDE: filed as #EXP-1941 and decided by Danny on 2026-11-18: calls are sealed with the chat's key. Built in two tracks, #EXP-1952 (TypeScript) and #EXP-1953 (Rust), PRs 1 to 12 landed dormant (Niach/exponential#3031 to Niach/exponential#3112); the switch, PR 13, has not landed. How the design was checked: L-82 to L-85.

### L-51 Authorization is tested where it runs

- Mistake: the rules were SQL where-clauses tested against a stub that ignores them; a voice Assistant was derived without membership and mirrored into a second row.
- Why: the stub kept the unit tests fast, and each new rule added another writer of the same fact.
- Rule: authorization is a pure function over loaded rows or is tested on a real database, and a derived fact is computed in one place.
- eDE: fixed, #EXP-1935 (Niach/exponential#3011): call authorization runs on a real database.

### L-52 A held hotkey has a liveness check, and something explicit keeps the process alive

- Mistake: "held" came from an OS release event that a device grab, a portal restart or a synthetic key can lose or fake. Whether the app stays alive behind its tray was decided in three places, one of them the toolkit's default.
- Why: the contract said which mechanism each OS uses, not what the app may assume about it.
- Rule: every hotkey event carries its origin (person or synthetic) and a hold is checked against device state; computer use and the hotkey share one input authority; one function decides quit.
- eDE: filed and fixed as #EXP-1938: closing the last window never quits by gpui's own rule (Niach/exponential#3019); an unbound portal shortcut says so, a key held into a computer-use hold is released, an agent's keystroke no longer fires the chord (Niach/exponential#3020); the eDE GlobalShortcuts portal answers only the shortcuts it bound (Niach/exponential#3073). A liveness check against device state is not filed.

## The phones

The orchestration of a call on the phones is under L-44.

### L-53 What depends on OS state is applied again on every change

- Mistake: Android routed the call's output once, so a headset that connected mid-call was ignored; the microphone service was skipped for the whole call when the app was not visible at the start, and nothing said so.
- Why: a changing environment was treated as a one-time check, and a fix closed an issue by avoiding the crash, not by deciding what the person gets.
- Rule: routing, service and session are a function of (call state, visibility, devices), re-applied on each event; a skipped protection is a state the person sees.
- eDE: filed and fixed as #EXP-1940 (Niach/exponential#3023): the output follows a headset mid-call, and a refused microphone service says so. Then #EXP-1942 (Niach/exponential#3025): the service starts at the tap, while the app is in front. A dropped call from Android is open, #FEED-268.

## Tests, CI and releases

### L-54 What CI runs is derived from the build graph

- Mistake: the PR gate did not run ePilot's tests at all, and hand-written path filters missed dependencies the eDE suite itself pins.
- Why: the gate was cut for speed, and each new dependency needed a filter edit nobody was forced to make.
- Rule: the set of suites comes from path dependencies and embedded files, and a gate test fails when the filter does not cover them.
- eDE: fixed for ePilot, #EXP-1905 (Niach/exponential#2955); the timing tests it exposed, #EXP-1927 (Niach/exponential#2996). The `ede` job now runs for what eDE builds from (#EDE-140, Niach/exponential#3151); the filter is still written by hand.

### L-55 The contract test between two components runs with the real ones

- Mistake: CI ran the lock tests without the real lock client, and the one test that runs ePilot against a real compositor returns early unless a variable is set that no workflow sets. A skipped test looked like a passed one. The macOS and Windows backend tests were compiled by no workflow.
- Why: the two workspaces build in separate jobs, so the place where they meet had no gate.
- Rule: the real client against the real server whenever either side changes; an env-gated test fails when CI says it is required; every OS's tests run on that OS.
- eDE: fixed. The lock, #EDE-83 (Niach/exponential#2965). The rest as #EDE-140: ePilot's live eDE test runs and survives a compositor restart (Niach/exponential#3138), the macOS and Windows tests compile and run (Niach/exponential#3146).

### L-56 A refusal test first shows the input landing

- Mistake: two refusal tests could not fail. They pressed a key that was still down from an earlier step, the input layer absorbed it, and the guard under test could be deleted without a red test.
- Why: negative assertions without a positive control, against a test client that saw presses only.
- Rule: the same input lands with the guard off, then is refused with it on; the test client records presses, releases, modifiers, buttons and axis; every input test ends with nothing down.
- eDE: fixed. The client records more since #EDE-73 (Niach/exponential#2939); the two tests were filed as #EDE-140 and can fail now (Niach/exponential#3137).

### L-57 A fake can fail, stall and refuse

- Mistake: the fake backend never failed, the fake shell drew every document, the fake services answered at once, the stub database ignored the where-clause. The paths that broke were the ones no fake could reach.
- Why: fakes were written for the happy path they made fast.
- Rule: each fake can inject a failure per step, and a contract is tested against the real consumer's behaviour.
- eDE: per defect, #EXP-1895 (Niach/exponential#2937), #EDE-110 (Niach/exponential#3002), #EXP-1935.

### L-58 A test does not compare today against a fixed date

- Mistake: a test dated its data relative to now against a fixed cutoff. It turned red on the release PR and stopped the cut of eDE 0.2.28 on 2026-11-17.
- Why: `host-spoofing.real-db.test.ts` seeded `first_seen_at = now - 20 days`, and migration 0213 recomputes only rows older than a fixed instant (2026-10-28 12:22 UTC). From 2026-11-17 12:22 UTC the seeded row was younger than that instant, the migration left it alone, and the test failed on every PR's gate.
- Rule: a test gets its clock as an input.
- eDE: fixed, #EDE-104: the seeded dates are anchored to the cutoff, not to now.

### L-59 A golden capture waits for the frame it asserts

- Mistake: a golden capture raced the first draw (the start menu was missing once and the picture passed on a rerun), and the golden gate tolerated 0.5 % of the pixels while its comments said a changed pixel fails.
- Why: capture and render were not tied to each other, and the tolerance was a shared default.
- Rule: a capture is taken after the frame that contains the state under test, and a tolerance is the measured noise, stated next to the test.
- eDE: fixed. The capture waits for the frame of every surface the view needs and fails rather than capture a half-drawn desktop (#EDE-106, Niach/exponential#3100). The tolerance was measured and set to one pixel in 10,000 plus 2 % faint (#EDE-140, Niach/exponential#3152); see L-77. Rerusted already compares what the compositor draws exactly (decisions.md, #RUST-21).

### L-60 A release is run in a real session first

- Mistake: four runs on a real session in a VM found what no headless test saw: the start menu without the keyboard (#EDE-86), the polkit prompt (#EDE-87), the first X11 window without keys (#EDE-88), the taskbar preview that closed the start menu (#EDE-89). Three of the four were already in the released 0.2.6.
- Why: the headless tests run no login session, no polkit of the distro and no real keyboard.
- Rule: before a release, the build is installed in a VM, driven with keys as scancodes, and checked against a fixed list (lock, prompts, focus, the journal); the installed build is proved by hash, not by version string.
- eDE: done by hand on 2026-11-17 (vm-run.md, compare.md, recheck.md, polkit-check.md on #EDE-69); fixes Niach/exponential#2974, Niach/exponential#2975, Niach/exponential#2981, Niach/exponential#2984. Rerusted has the rig for it (tier 3, #RUST-21).

## Error reports

### L-61 Every binary and every child reports, and a dead feature is an error

- Mistake: only three of eDE's seven binaries had error reporting (the compositor, `exponential-session`, `exponential-services`), and neither worker child had it (#ERR-29). Push to talk panicked deterministically from 2026-11-09 and no report exists. Voice and computer use logged every dead end as a warning, and only errors become reports.
- Why: each `main()` copied its own set-up, a worker branch above it opted out silently, and an empty error project looked like health.
- Rule: one process entry that every binary and re-exec mode goes through, checked by a test that lists the binaries; a child hands its exit status to its parent as data; feature health is a typed state whose step into "dead" is what reports.
- eDE: filed as #ERR-29 and fixed: the shell, the sealed surfaces, the portal and the supervisor report like the compositor (Niach/exponential#3155); a speech worker's crash is reported with what ended it, and Voice's dead ends are errors (Niach/exponential#3171); computer use's dead ends are errors (#ERR-31, Niach/exponential#3157). The panic itself is fixed, #EXP-1901 (Niach/exponential#2941). See L-81.


## The agent spaces core as built

eDE built the core between 2026-11-17 and 2026-11-19 from one design (`core-design.md`), a synthesis of three drafts, one of them Rerusted's shape ported (`design-rerusted.md`), after two adversarial checks with 31 problems: 1 blocking, 7 major, 23 minor (`core-design.md:5`). It landed as about 30 PRs under #EDE-125 and #EDE-126. Step F3 (an Xwayland, a bus and a systemd slice per space) is #EDE-138, in review; offering spaces to people (F5) waits.

### L-62 The seat and output filter goes into the library, pinned by rev

- Mistake: the closed draft Niach/exponential#1536 parsed a global's space from Smithay's debug text (L-41), and a one-seat toolkit sees any second seat: gpui rebinds to the last `wl_seat` announced (`core-design.md:230`).
- Why: Smithay has no per-client filter for the seat and output globals.
- Rule: fork the library with the smallest commits that make it right, pin the fork by rev and never a floating branch, and drop the fork when upstream has them. eDE's fork, branch `client-filter-65aa86f` of Niach/smithay, has three: a client filter for `wl_seat` and `wl_output`, `SeatState::remove_seat` and `XwmHandler::dnd_seat` (L-63, L-64).
- Test: a client of the person's socket never sees a space's seat or output; on close it hears two `global_remove` for names it never saw and keeps working (gpui ignores them, `core-design.md:214`; other toolkits: one hand check).
- eDE: done. `apps/ede/Cargo.toml:16-27` (rev `bacaea68`), the person's globals filtered to the person's clients (#EDE-125 step 1, Niach/exponential#3046).

### L-63 A seat is removed on a condition, never by a timer

- Mistake: the first drafts removed a closed space's seat 2 s after close, or pooled four seats.
- Why: Smithay's seat list is only ever pushed, and a handle of a missing seat panics (`core-design.md:47`). A pooled seat changes owner, so a late event names the wrong desk (`core-design.md:243`).
- Rule: remove a seat only when nothing but the list and the caller holds it, try again each tick, and keep a seat that stays shared (logged once): a leaked seat is cheap, a panic is not (`core-design.md:122`, `:260`).
- eDE: done. A space closes and leaves nothing (#EDE-125 step 5, Niach/exponential#3058); in the VM the space's directory was gone 0.6 s after destroy (`space-proof-master.md`, B7), after a first run had left it behind (`space-proof.md`, B7).

### L-64 Before a second seat, audit every place the library scans all seats

- Mistake: the design left the X11 drag to step F3. Smithay's XWM takes the newest left-button grab over every seat, so a person's X11 drag would land on an agent's seat whose button went down later (`core-design.md:48`).
- Why: one seat made "any seat" and "the seat" the same; an XWM per space in F3 would have made it two-way.
- Rule: list every cross-seat path of the library first (in Smithay 65aa86f this one is the only one), and fix it in the fork before the second seat exists.
- eDE: done. The verdict F2 (`core-design-verdicts.md:27`) moved it into the fork; a person's X11 drag stays on their seat (#EDE-125 step 11, Niach/exponential#3077).

### L-65 A connection's scope is fixed at hello

- Mistake: the design derived a connection's desk from its grant at each call and at drop. A foreground run revokes its grant before its hold connection drops, so the drop would find no grant and the person's input would stay held until Esc.
- Why: on master the scope was never needed at drop, so nobody had asked what it is once the grant is gone.
- Rule: resolve the scope once when the connection says hello (person, space n, or unbound), store it on the connection, and read only that; a revoked or expired grant still releases what its connection holds.
- Test: hold input, revoke the grant, drop the connection: the person's input is free and no driven key is down.
- eDE: the one blocking problem of the checks, F1 (`core-design-verdicts.md:21`); fixed as designed (#EDE-126 step 3, Niach/exponential#3053).

### L-66 A tag never falls back to the person

- Mistake: the natural default was "a client with no state is the person's". F3's Xwayland would then have been the person's by accident (`core-design-verdicts.md:239-243`).
- Why: the person was the only desk, so the default was always right before.
- Rule: client, surface, seat and window carry their desk; what a space tagged never enters the person's branch, also after the space closed; a client with no tag is nobody's and sees no person global; nothing of a space is listed, emitted or drawn on the person's side (`core-design.md:7-11`, `:263`).
- eDE: done (#EDE-125, #EDE-126).

### L-67 Every handler that takes a surface resolves its desk; a miss is refused

- Mistake: the guard list named the xdg, compositor, seat and selection handlers and missed three: `new_fractional_scale` fell back to the person's first output and told a space the person's monitor scale, `maximize_request` configured without a window, and `inhibit` stored any surface (`core-design-verdicts.md:39-43`, `:311`; `core-design.md:295`).
- Why: a guard list is written from memory, and every handler written for one desk falls back to "the" desk.
- Rule: a handler that receives a client or surface looks its desk up and refuses on a miss, never `outputs().next()`.
- Test: one request per global a space client sees, each checked against the person's state (`core-design-verdicts.md:207`).
- eDE: the three went into the guards of step 8 (`core-design.md:194`): nothing a space client asks moves the person's desk (#EDE-126, Niach/exponential#3070).

### L-68 The person's privileged path refuses every other desk by type

- Mistake: the shell's bindings path checked only that the grant was the root's, then drove the person's keyboard and pointer whatever desk the request resolved to. Only the space scope's refusal stood in front of a mix (#EDE-136). The second "refusal" in the design was a comparison with a session name that `MintGrant` accepted for anyone (`core-design-verdicts.md:75-79`).
- Why: before spaces, every desk was the person's, so "root" and "person" meant the same.
- Rule: a path that drives the person's input takes the person's desk as a type, not a check beside it, and refuses (Forbidden) whenever the resolved desk is another; root is checked by token, not by name.
- eDE: fixed, #EDE-136 (Niach/exponential#3115).

### L-69 Dormant first, one switch last

- Mistake: in the first PR order, a space could be created over IPC before the clipboard guards had landed, so a copy inside a space would have moved the person's clipboard and taken Xwayland's CLIPBOARD (verdict F3, `core-design-verdicts.md:33`).
- Why: each PR was safe alone; the order between them was not checked.
- Rule: name the gate PR, the first one that lets anything outside a test reach the new path, and land it only after every guard; everything before it ships dormant. The acceptance test lands early, ignored, so its helpers serve every later PR (`core-design.md:252`, `:267`).
- eDE: done twice. Spaces: the acceptance test first (Niach/exponential#3081), launch into a space behind `features: ["spaces"]` (Niach/exponential#3091), ePilot's space target behind `EXPONENTIAL_COMPUTER_SPACE=1` (`space-proof.md`). The call seal: 17 PRs landed dormant (Niach/exponential#3031 to Niach/exponential#3112, "the host socket seals, dormant", Niach/exponential#3085) before its Enable, PR 13 (`call-seal-design.md:325`).

### L-70 Grants and filters stop accidents, not a hostile process of the same user

- Mistake: the core's allowlist could look like isolation. A launch into a space runs as the same user, the root token is a file in the runtime directory (mode 0600), and the person's Wayland socket and every space's socket are at known paths: one launch reaches the person's seat and screen and any other space (verdict D14, `core-design-verdicts.md:245-249`).
- Why: everything below the user boundary is one principal to the OS.
- Rule: say in the design that grants and filters guard against accidents until a sandbox exists, so consent is not built on them; a launch never gets `WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, `DBUS_SESSION_BUS_ADDRESS` or a token of the person (L-32).
- eDE: written into the design; a launch gets the space's own runtime directory and never the person's bus (`core-design.md`, decisions after the checks). The token file is unchanged (L-32).

### L-71 A space's processes must not be the person's to system-wide services

- Mistake: a space had its own session bus and runtime directory, and that looked like enough. In the VM `pkexec true` inside a space raised the person's polkit prompt within 6 s and took their keyboard; the person's next typed line and Enter went into the password field and were submitted (`space-proof.md`, B8; #EDE-127). Snaps refused to start and Ptyxis could not spawn its shell, because nothing on the space's bus is a systemd user manager (`space-proof.md`, B3; #EDE-128).
- Why: polkit, logind, snap and systemd find their caller by login session or cgroup over the system bus, and a space's processes kept the person's `XDG_SESSION_ID`, `XDG_SEAT` and `XDG_VTNR`. The design had named polkit as a risk it did not fix (`core-design.md:216`).
- Rule: list every system-wide agent that decides by session, seat or cgroup (polkit, logind, the keyring, the portals, the systemd user manager, the sound server, rfkill) and give each a desk: it refuses a space or serves the space's own instance. A space's processes leave the person's session.
- Test: from a space, `pkexec`, `loginctl lock-session`, `systemctl suspend`, opening the microphone: nothing reaches the person; a snap starts.
- eDE: fixed one by one after the VM found the first two. polkit refuses a request from a space before anything maps (#EDE-127, Niach/exponential#3133); a space has its own systemd user manager and slice (#EDE-138, Niach/exponential#3161, Niach/exponential#3174, Niach/exponential#3175); nothing in a space locks, logs out, suspends or reboots the person's session (#EDE-146, Niach/exponential#3178), opens their microphone, camera, radio switches or input devices (#EDE-148, Niach/exponential#3176), or reaches their sound servers (#EDE-147, Niach/exponential#3179).

### L-72 The proof runs from the person's side, on the base it lands on

- Mistake: the first space proof ran on a stack that was not rebased onto master; its two failures (N1, N2) came from the base, not from spaces (`space-proof.md`). Two focus failures were the test's own: a `gnome-clocks` instance from an earlier check was still alive (`space-proof.md`, F3).
- Why: a long stack drifts from master, and a single-instance app survives between checks.
- Rule: measure what the person feels (every typed key arrives, the largest pause, focus, pointer and clipboard sampled each second, screenshots), start each check from a fresh session, and run the proof again on master after the last PR lands.
- eDE: done. `space-proof.md` (stack `cbbce02d03`, 2026-11-17) and `space-proof-master.md` (master `94eb3510`, 2026-11-19), both from a release `.deb` checked by sha256, the person's keys as scancodes.

## Real runs

### L-73 A bug found on a real session gets a headless test that fails on master first

- Mistake: three release blockers of 2026-11-17 came from the VM, not from CI: a click on a toast closed the maximized xterm under it (#EDE-119), the only X11 window came back from minimize without the keyboard (#EDE-122), a click inside the start menu focused the window under it (#EDE-123). The headless test and the VM also disagreed the other way once: on `fb1540fc6f` the headless test failed while the VM passed, because the VM's restore came 2 s after the minimize (`release-check.md`, "Two windows on the old build").
- Why: #EDE-88 gave every X11 window a title bar, which put a close button under every toast's X; no headless test had a real X11 client in that place.
- Rule: each issue a real run finds names the headless test that must fail on master, and the fix lands with it; where the two disagree, the difference (timing, a real client) goes into the test.
- eDE: done for all three (Niach/exponential#3018, Niach/exponential#3021); see L-08.

### L-74 A release check is repeated per build and says what changed by hash

- Mistake: a release candidate took three builds of master before it passed (`18dcb76265` failed X1, `fb1540fc6f` failed N1, `43f99d26c9` passed; `release-check.md`). On 0.2.29 the package still read 0.2.28-1, and the VM snapshot was a saved running session, so the old processes kept running after `dpkg -i` until a reboot (`release-check-0.2.29.md`).
- Why: version strings and "installed" do not prove what runs.
- Rule: after each fix, re-check as a new build: repeat the checks whose code changed, prove which binaries changed by sha256 and say why the rest need no repeat; reboot or log out after the install.
- eDE: done by hand for each candidate since (`release-check.md`, `release-check-0.2.29.md`). L-60 is the first half of this rule.

### L-75 An intermittent bug gets a fixed script, run on the old and the new build

- Mistake: a taskbar click on a minimized window sometimes did nothing: 2 of 18 clicks, then 1 of 11, then 4 of 7, each by a slightly different hand method, and nobody could say whether the released build had it (#EDE-124).
- Why: "sometimes" by hand measures the method as much as the bug.
- Rule: write one script for the steps, run it the same number of times on the released build and the candidate, and give both counts.
- eDE: with a script the released 0.2.28 missed 39 of 40 and the candidate 0 of 40, after the fix (#EDE-124, Niach/exponential#3106; `release-check-0.2.29.md`).

### L-76 A choice no spec makes is decided before it is built

- Mistake: the start menu had the keyboard again (#EDE-86), but Enter launched nothing and the arrow keys selected nothing. It was seen in four VM runs before it was fixed: the re-check that found it (`recheck.md`), still there in the text re-check (`release-check.md`), passed in `space-proof-master.md` and in `release-check-0.2.29.md`.
- Why: the menu's document said it opens "with the keyboard" and defined no selection, so it needed a decision that added behaviour (#EDE-92).
- Rule: a real run that finds behaviour nobody specified files it as a decision with a proposal from the model desktop (Mint launches the first match on Enter), and the fix waits for it, not for another run.
- eDE: decided and fixed, #EDE-92 (Niach/exponential#3062); its scroll follow-up #EDE-133 (Niach/exponential#3083).

## Tests and the gate

### L-77 A golden can record a defect as the truth

- Mistake: the sound applet's popup never opened (#EDE-93), and the gate was green: the fix re-anchored four applets' popups and changed their four goldens with it, each from about 7 KB to 9 to 14 KB (Niach/exponential#3103). Nine more goldens were stale since a token change on 2026-11-09 that the 0.5 % tolerance never saw, and some had been recorded with the machine's own Inter font (#EDE-140, Niach/exponential#3152).
- Why: a golden is recorded from what the code draws today and reviewed as a file, not as a picture of the intended state.
- Rule: a golden is looked at as an image against what the view must show when it is recorded or changed; the capture uses its own fonts; the tolerance is the measured noise between runners (L-59).
- eDE: done. eDE's goldens are held to one pixel in 10,000 plus 2 % faint, and all 21 were re-recorded from master (#EDE-140, Niach/exponential#3152); filed: #EDE-155 (the shell draws with the machine's Inter when it is installed).

### L-78 A capture is drawn at the output's own scale

- Mistake: an output capture drew every element at scale 1 on a fractional-scale output, while positions were scaled. At 1.5 the capture differed from the screen by 49,396 to 303,895 pixels, against 0 at scale 1 (`release-check.md`, "C3 at a scale other than 1"; #EDE-113).
- Why: the off-screen damage tracker was built with scale 1.0, and the goldens all run at scale 1.
- Rule: capture through the screen's own path at the output's scale (L-35), and keep one test at a fractional scale; list what a capture leaves out on purpose.
- eDE: fixed, #EDE-113 (Niach/exponential#3094); the 1,886 pixels left are the title text, which captures leave out on purpose (`release-check-0.2.29.md`).

### L-79 A test owns its resources and asks the OS for them

- Mistake: two X11 tests started a private Xvfb at a display number derived from the pid and accepted the first server they reached. When another test binary's Xvfb held the number, theirs failed, the probe connected to the foreign one, and it later vanished: 12 and 7 failures in 4 × 60 runs (#EXP-1970). A log test counted records of other threads (#EXP-1964).
- Why: a fixed or derived number is unique only on an idle machine; a self-hosted runner runs several jobs at once.
- Rule: a test takes what the OS hands out (Xvfb `-displayfd`, port 0, a temp dir) and checks that what it reached is its own; it counts only its own records.
- eDE: fixed, #EXP-1970 (Niach/exponential#3088), #EXP-1964 (Niach/exponential#3068).

### L-80 A flake under load asks first whether the product has the same fixed wait

- Mistake: with the display collision fixed, the clipboard round trip still failed now and then under load: the X11 exit handover waited a fixed 100 ms for the clipboard manager, so a slow manager saved nothing (4 in 960 runs; #EXP-1973). A lock test waited 100 ms for a client to go (#EDE-170).
- Why: a fixed window passes on a fast machine and is easy to blame on the test.
- Rule: replace a fixed wait by the event it stands for, with a bound, in the product if the product has it; a load loop (several parallel runs at high load) is the proof.
- eDE: fixed in the product, #EXP-1973 (Niach/exponential#3095): the handover waits for the manager's answer; #EDE-170 (Niach/exponential#3196).

## Error reports

### L-81 A report path is tested end to end, from every thread and after a rebuild

- Mistake: beyond L-61, a desktop panic reached GlitchTip from the UI thread only; other threads kept a closed client after it was rebuilt (the Settings switch, a new config). The voice crates had 0 `log::error!` calls, and a speech worker killed with SIGSEGV gave the host no signal and no error (#ERR-29, the verification on 2026-11-19).
- Why: nobody had ever forced a panic and looked for the report.
- Rule: each process and each terminal state has a test that forces it against a stub collector and finds the report; the test runs against master's behaviour first and must fail there.
- eDE: fixed, #ERR-30 (Niach/exponential#3156), #ERR-29 (Niach/exponential#3155, Niach/exponential#3171), #ERR-31 (Niach/exponential#3157); release builds keep their libraries' log lines (#ERR-32, Niach/exponential#3183).

## The call seal

#EXP-1941 sealed a call's transcript and audio with the chat's key (L-50). Its design was a synthesis of two drafts, checked in three adversarial rounds on 2026-11-18 (`call-seal-verdicts.md`).

### L-82 A home-made protocol is attacked until a round finds nothing in what the last one changed

- Mistake: round one (cryptography; feasibility) found 1 blocking and 7 major problems; round two found 10 major, which were 7 different ones, "all in the parts changed after round one" (`call-seal-verdicts.md:301`, `:438`); round three, narrow on what round two had changed, found 3 more (`call-seal-verdicts.md:684`).
- Why: each round's fixes were new and unattacked (`call-seal-verdicts.md:900`).
- Rule: after each round, attack exactly what it changed; when the fixes keep making new holes, adopt a published pattern instead of patching the table again. The call seal took Noise NNpsk2's rule for the responder's message and lost a state and three rules (`call-seal-verdicts.md:902-910`).
- eDE: done; "the handshake and the call id are ready to build" (`call-seal-verdicts.md`, third round). Rerusted's identity spikes (#RUST-39) are a place this applies.

### L-83 A capability is advertised when its holder starts, and withdrawn when it stops

- Mistake: the hosts registered their caps at launch, before the key holder existed, and registered again only when the agent list changed. So the `call-seal` cap never reached the server from a daemon and only by a race from the IDE: after Enable two updated ends would still have talked in clear, silently (D1, `call-seal-verdicts.md:543`, `:554-557`).
- Why: caps were computed once at start, when everything they named used to exist already.
- Rule: a cap is a register trigger: the component that makes it true registers it, and the one that stops withdraws it; a switch is checked against a real host before it is turned on.
- eDE: built into the cut ("the cap is a register trigger", `call-seal-design.md:205`); Enable has not landed.

### L-84 Say exactly what a seal protects against

- Mistake: four headers claimed the relay carries no session content while calls crossed it readable (#EXP-1941). The seal then had to state its limits: a server that lies about a chat's key can read the chat and a sealed call (#EXP-1950), and with a clear call id the relay can still take the caller's place (`call-seal-design.md:216-218`).
- Why: a principle in comments is never checked against the code.
- Rule: the code and docs say which party a seal protects against (here the relay and a server that only reads, accepted by Danny on 2026-11-18, #EXP-1950) and which it does not; a canary test in the relay proves it reads nothing it should not.
- eDE: headers corrected in PR 1 (Niach/exponential#3031), the relay's canary test in PR 2 (Niach/exponential#3034); #EXP-1950 is in the backlog.

### L-85 What the server mints vouches for a request before anyone acts on it

- Mistake: the host acted on the ring's agent id, which the relay writes, before any key was proven: it could resolve a chat, create and key one, answer pending wraps (`call-seal-design.md:209`; ID8, `call-seal-verdicts.md:840`).
- Why: the relay was trusted to route, and routing data looked harmless.
- Rule: the server mints an id that carries every field the other end acts on (here the agent, in both modes) under a MAC; the host asks for the ticket first and does nothing for a ring the server did not mint.
- eDE: done, #EXP-1952 PR 4a (Niach/exponential#3035): `callHostTicket` answers only a ring the server minted.

## Still open

What this document cannot say yet.

- Agent spaces F3 (#EDE-138, in review) and F5, offering spaces to people. Whether GPU apps start in a space before step F7 needs a real GPU, not a VM (`core-design.md:229`).
- How an agent acts on a lent window (F8): view only is the recommendation, because a second seat breaks one-seat toolkits (`core-design.md:230`).
- The call seal's Enable (PR 13) and Mandatory, and #EXP-1950 (signed key wraps).
- The token file (L-32), run-scoped cleanup (#EXP-1917), the display change in screen memory (#EXP-1913), and the shared state machines and registries L-04, L-08, L-23, L-27, L-29 call for: not filed or in the backlog.
- Real runs still owed: a call from an Android phone (#FEED-268), push to talk on a real build, a GPU client in a capture on real hardware, two monitors while locked.
