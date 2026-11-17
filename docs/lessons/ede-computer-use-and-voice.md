# Lessons from eDE: computer use and Voice

Status: first part, as of 2026-11-17 19:33 UTC; a second PR appends the background core and the real runs
Source: #EDE-118, from the review #EDE-69

What the team learned building computer use and Voice in eDE (Exponential Desktop, repository Niach/exponential), so Rerusted has them right from the start. Each rule gives the mistake eDE made, why it happened, the design rule that prevents it, and its sources. This is not a spec: `docs/spec/06-*.md` (voice) and `docs/spec/07-*.md` (compositor) stay reserved, and a design is checked against these rules when they are written.

## How to read it

- The review #EDE-69 ran on 2026-11-17: 28 review areas, 292 findings, 84 raw lessons. They say the 61 things below. No adversarial verification ran on the findings; the coordinator read 19 of them against the code (#EDE-69).
- Issues (#EDE-nn, #EXP-nn) are issues of the Exponential tracker. Pull requests are written `Niach/exponential#nnnn`. A rule whose findings have no issue yet names the review, #EDE-69.
- "eDE" under a rule is the status in Niach/exponential when the review state was archived: **fixed** (the PR is on master), **in work** (an issue, a PR open or none yet), **filed** (an issue only), **not filed**, or **needs a redesign**.
- Four rules are decisions Danny took on 2026-11-17, marked so. They are decisions for eDE. They are not rows in [decisions.md](../decisions.md): that file holds what was decided for Rerusted.
- Background computer use does not work in eDE. On 2026-11-17 only the preparation landed; see [Spaces](#spaces-and-why-background-mode-did-not-fit).

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
- eDE: fixed. #EDE-71 (Niach/exponential#2932), #EDE-86 (Niach/exponential#2974). Open decision: #EDE-107 (a prompt over a fullscreen window).

### L-07 A focus request names its seat and its origin

- Mistake: `focus()` could not tell a new window, a client request, the shell, an agent and the person's click apart, and its target type could not express X11. The first X11 window of a session read `focused: true` and got no key until it was clicked.
- Why: single-seat use needed no caller; X11 was added beside the Wayland path.
- Rule: every focus, raise and placement request carries a seat and an origin, and the focus target is one enum over Wayland, X11, layer and lock surfaces, so the protocol handoff exists once.
- eDE: fixed for X11, #EDE-88 (Niach/exponential#2981). The seat and origin need the Desk (#EDE-90).

### L-08 What is drawn, what is hit and where the pointer is come from one list

- Mistake: drawing order, hit-testing and pointer focus were three hand-written walks that disagreed: a press on a title-bar button reached the window under it, and a hidden panel took clicks under a fullscreen window. Window lifecycle lived in four containers with copied fix-ups.
- Why: frames, popups and "fullscreen draws alone" were added to the renderer only.
- Rule: one ordered list of regions per output and one window registry with an explicit state; rendering, hit-testing, captures and the pointer refresh derive from them.
- eDE: fixed for the hit-testing defects, #EDE-103 (Niach/exponential#2994). The single registry is not filed (#EDE-69).

### L-09 System chrome is one overlay primitive that takes no focus and no click

- Mistake: the computer-use banner, the call pill and the push-to-talk capsule were pop-up windows assembled per caller; on Wayland each was a normal window that took focus. The indicator lived in whichever process hosted the run.
- Why: it was built for an IDE on X11, and no test opened these windows on a Wayland compositor.
- Rule: one overlay surface with a stated contract (no keyboard, no pointer, top layer, never on a fullscreen display), drawn by the component that owns the hold; where the contract cannot be kept, it does not open.
- eDE: fixed. #EXP-1912 (Niach/exponential#2964, Niach/exponential#2966, Niach/exponential#2969), #EXP-1923 (Niach/exponential#2982). The pill was pictured from a specimen, no real call ran, and moving the indicator into the compositor is not filed (#EDE-69).

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
- eDE: needs a redesign (one clipboard, cause 3 below). Narrowed by #EXP-1920 (Niach/exponential#2970, Niach/exponential#2971); a killed host still loses the restored X11 clipboard (#EDE-69).

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
- eDE: partly fixed, #EXP-1910 (Niach/exponential#2960). Filed: #EXP-1917 (run-scoped cleanup).

### L-22 One authority says a session ended

- Mistake: the device decided from local outcome strings that a session ended and destroyed volumes and simulators, while the server kept root chats and lost runs alive. The cleanup lived in one of the two host types.
- Why: the end rule was copied to the device when ends were simple and never followed the server's exceptions.
- Rule: the device destroys only on the synced row's verdict, in one idempotent sweep that every host type runs and that checks liveness before each step.
- eDE: fixed. #EXP-1909 (Niach/exponential#2957), #EXP-1911 (Niach/exponential#2959).

### L-23 One runner per desktop owns runs; UIs are its clients

- Mistake: a local session's run state belonged to whichever process won a file lock. The store row was a mirror nobody reconciled, so a stopped or crashed run stayed `busy`.
- Why: the shell and the app each embed the runner; the lock was added to stop double runs, not to define an owner.
- Rule: one service owns runs, tools and change events, and persisted run state carries an owner lease (pid and epoch) so a stale one is detected on takeover.
- eDE: not filed (#EDE-69).

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
- eDE: the panics are fixed, #EDE-75 (Niach/exponential#2935), #EDE-89 (Niach/exponential#2984), #EXP-1901 (Niach/exponential#2941); compare.md on #EDE-69. The single owner and the calls on the UI thread are not filed (#EDE-69).

### L-29 Every action a document can fire has a typed handler in every host

- Mistake: the shared chat document fired actions the eDE host only partly implemented: `stop` was an empty arm, a failed send lost the composer's text, a missing argument defaulted to "open".
- Why: documents are JSON, handlers match on strings, and the tests grep for names.
- Rule: the host-function contract is generated from one spec, handlers are exhaustive at compile time, and a fixture replays each document's events against the real host.
- eDE: not filed (#EDE-69).

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
- eDE: fixed for the lock, #EDE-72 (Niach/exponential#2942), #EDE-79 (Niach/exponential#2949). The review state does not say whether the token file changed (#EDE-69).

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
- eDE: fixed for the defects, #EXP-1908 (Niach/exponential#2958). Filed: #EXP-1913.

### L-35 The tested backend and the real backend share the pipeline

- Mistake: headless had an unconditional frame clock, no damage and no flip queue, and captures used a second renderer that could not read what the screen renderer shows. On real hardware the windows of GPU clients were missing from captures.
- Why: the real backend grew from an example as one file; headless was written for pixel tests.
- Rule: one per-output frame scheduler (events in, actions out) that every backend drives, and capture through the renderer that draws the screen; state is updated at the event it is named after (presented means the flip was acknowledged).
- eDE: fixed. #EDE-100 (Niach/exponential#2998), #EDE-102 (Niach/exponential#2993), #EDE-109 (Niach/exponential#3003). Filed: #EDE-113. Not yet seen on a real GPU (#EDE-69).

### L-36 Capturer health is a state the person sees

- Mistake: a failed grab was a debug log, the status knew only off, active and paused, and eDE became a capture platform through an unrelated probe, without a spec or a test.
- Why: the capturer trait returns frames or strings; nothing models "the session ended".
- Rule: health is a state (capturing, paused, failed with a reason, reopening), and a platform path nobody declared stays off.
- eDE: not filed (#EDE-69).

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

### L-38 Model the target from day one

- Mistake: one seat, one focus, one pick, one banner, one lock, one clipboard, as singletons and process statics. Whether a fullscreen window was the agent's or the person's was guessed.
- Why: foreground-only was the first product, so singletons were the cheapest shape and the backend got no notion of who acts where.
- Rule: a target is a seat with its outputs, focus, selection and cursor; every backend call, gate and indicator takes one, and the person's seat is one such target.
- eDE: needs a redesign; the preparation above is its first half. Rerusted has the type (`Desk`, #RUST-23).

### L-39 The hold is not isolation

- Mistake: the only thing between the agent and the person was the hold, which mutes the person. So "the person types while the agent works" is ruled out by construction, and a locked screen stops the agent too.
- Why: the hold answered the foreground case and was then the only tool at hand.
- Rule: an agent space takes no hold and no Esc; the lock and the hold touch the person's desk only.
- Test: the acceptance test of the plan, section (d): one client on the person's socket, one on the space's; 200 person keys interleaved with 200 agent keys, pointer moves, a click and a clipboard set in the space. The person's client got exactly its 200 keys in order, the space's exactly the agent's; the person's focus, pointer and selection are unchanged; the hold was never on; a capture of the space shows the space's client and one of the person's output does not; locking leaves the space running.
- eDE: needs a redesign (#EDE-69).

### L-40 Every window and output has an owner, and readers say whose they want

- Mistake: the shell, screen memory, the portal pickers and Voice took every window as the person's.
- Why: there was nobody else.
- Rule: window and output records carry their desk; a reader asks for the person's explicitly, and a capability checks the space.
- eDE: fixed in the protocol, #EDE-91 (Niach/exponential#2987). Left for the core: the portal pickers and `exponential-ctl` list everything, and the focus capability does not check the space (#EDE-69).

### L-41 A client's view of seats and outputs is filtered by the library, not by tricks

- Mistake: the closed draft Niach/exponential#1536 added spaces beside the single-seat compositor: a second struct in a new file of 654 lines, a global's space parsed from the library's debug text, hand-written dispatch in place of the stock delegates, a new renderer per capture, no XWayland and a shared session bus.
- Why: the compositor had no place for a second seat, and the library had no filter for the seat and output globals.
- Rule: fix the library (a filter commit, #RUST-33) and keep one type for the person's desk and a space; each space has its own XWayland and session bus.
- eDE: needs a redesign. Rerusted has it (#RUST-23); the filter is required because gpui binds the last seat advertised (#RUST-36). Caveats from the plan: the commit is on Smithay 0.7.0 and eDE pins 0.8, and the spike has no DRM, no dmabuf and no wire protocol.

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
- eDE: in work for the daemon's defects, #EXP-1930. One host for the IDE and the daemon, and the phones' orchestration, are not filed (#EDE-69).

### L-45 The speech engine publishes one explicit state

- Mistake: `Voices: []` meant no GPU, a retry pending or a failed download. Each host guessed; the daemon turned a transient state into a permanent verdict and advertised "ready" from a probe whose worker had exited.
- Why: events were added per UI need, and the worker protocol assumed a resident worker.
- Rule: one state (starting, downloading, ready, retrying with time and reason, no GPU, failed); a transient state never looks like a verdict, and an advertised capability is proved again when the call is answered.
- eDE: in work. #EXP-1930, #EXP-1934 (Niach/exponential#3010, open).

### L-46 Desired state is not a job in the work queue

- Mistake: the voice choice travelled as a job in the same queue as the work; a latest-wins loop dropped it, so a Prepare followed by a Speak lost the voice.
- Why: the loop could not be tested, because it ran only after a real GPU bring-up.
- Rule: voice and language live in their own cell that each work item reads when it starts; protocol logic is never welded to hardware bring-up.
- eDE: in work, #EXP-1934 (Niach/exponential#3010, open).

### L-47 A shared cache has one owner, and a fallback decides on proof

- Mistake: the on-disk speech store was guarded by in-process mutexes and cleaned by whichever host thought it owned it; the player fallback relied on which call returned first.
- Why: it was designed for one worker per machine; the daemon became a second host later.
- Rule: one owner, or an OS lock with unique temp names and atomic renames; a fallback decides on bytes consumed or an exit status.
- eDE: in work, #EXP-1934 (Niach/exponential#3010, open).

### L-48 Whoever holds a device has a wall-clock deadline

- Mistake: a lost microphone was noticed only when the audio channel closed, and every time limit ticked in received samples. A silent stream never closed and never timed out.
- Why: the first capture was a recorder process that ends with EOF on any failure.
- Rule: device loss is an explicit event, limits run on the clock, one thread owns the listening state, and every request id gets exactly one terminal event (heard, empty, failed, replaced).
- Test: a fake device that stalls, closes and refuses to open.
- eDE: in work, #EXP-1937 (no PR yet).

### L-49 One reducer owns the voice session; what a call borrows is a guard

- Mistake: speaking, call phase and push-to-talk arming were written from many call sites and spread by repaint observers. What a call borrowed was given back by an unrelated 30 s heartbeat, and "thinking" ended only when a reply was spoken.
- Why: push to talk, calls and per-agent voices were each bolted onto the speaker entity.
- Rule: every transition is an event from a pure step; microphone, hotkey and voice are guards released when the call ends; busy and queued come from the session's turn state, and every waiting state has an exit for "nothing came".
- eDE: in work, #EXP-1936 (no PR yet).

### L-50 The relay forwards only sealed bytes

- Mistake: a call's transcript crosses the relay as readable JSON and its audio as plain Opus, though the relay claims to carry no session content.
- Why: calls were added as a second protocol beside the sealed tunnel; the principle lived in comments.
- Rule: one frame type of opaque sealed bytes plus ids and sizes, so a readable payload cannot be added without changing the type.
- eDE: not fixed and not filed; the review kept it as an open decision (#EDE-69).

### L-51 Authorization is tested where it runs

- Mistake: the rules were SQL where-clauses tested against a stub that ignores them; a voice Assistant was derived without membership and mirrored into a second row.
- Why: the stub kept the unit tests fast, and each new rule added another writer of the same fact.
- Rule: authorization is a pure function over loaded rows or is tested on a real database, and a derived fact is computed in one place.
- eDE: in work, #EXP-1935 (Niach/exponential#3011, open).

### L-52 A held hotkey has a liveness check, and something explicit keeps the process alive

- Mistake: "held" came from an OS release event that a device grab, a portal restart or a synthetic key can lose or fake. Whether the app stays alive behind its tray was decided in three places, one of them the toolkit's default.
- Why: the contract said which mechanism each OS uses, not what the app may assume about it.
- Rule: every hotkey event carries its origin (person or synthetic) and a hold is checked against device state; computer use and the hotkey share one input authority; one function decides quit.
- eDE: not filed (#EDE-69).

## The phones

The orchestration of a call on the phones is under L-44.

### L-53 What depends on OS state is applied again on every change

- Mistake: Android routed the call's output once, so a headset that connected mid-call was ignored; the microphone service was skipped for the whole call when the app was not visible at the start, and nothing said so.
- Why: a changing environment was treated as a one-time check, and a fix closed an issue by avoiding the crash, not by deciding what the person gets.
- Rule: routing, service and session are a function of (call state, visibility, devices), re-applied on each event; a skipped protection is a state the person sees.
- eDE: not filed; needs a real call from an Android device (#EXP-1889, #EDE-69).

## Tests, CI and releases

### L-54 What CI runs is derived from the build graph

- Mistake: the PR gate did not run ePilot's tests at all, and hand-written path filters missed dependencies the eDE suite itself pins.
- Why: the gate was cut for speed, and each new dependency needed a filter edit nobody was forced to make.
- Rule: the set of suites comes from path dependencies and embedded files, and a gate test fails when the filter does not cover them.
- eDE: fixed for ePilot, #EXP-1905 (Niach/exponential#2955); the timing tests it exposed, #EXP-1927 (Niach/exponential#2996). The derived filter is not filed (#EDE-69).

### L-55 The contract test between two components runs with the real ones

- Mistake: CI ran the lock tests without the real lock client, and the one test that runs ePilot against a real compositor returns early unless a variable is set that no workflow sets. A skipped test looked like a passed one. The macOS and Windows backend tests were compiled by no workflow.
- Why: the two workspaces build in separate jobs, so the place where they meet had no gate.
- Rule: the real client against the real server whenever either side changes; an env-gated test fails when CI says it is required; every OS's tests run on that OS.
- eDE: fixed for the lock, #EDE-83 (Niach/exponential#2965). The rest is not filed (#EDE-69).

### L-56 A refusal test first shows the input landing

- Mistake: two refusal tests could not fail. They pressed a key that was still down from an earlier step, the input layer absorbed it, and the guard under test could be deleted without a red test.
- Why: negative assertions without a positive control, against a test client that saw presses only.
- Rule: the same input lands with the guard off, then is refused with it on; the test client records presses, releases, modifiers, buttons and axis; every input test ends with nothing down.
- eDE: the client records more since #EDE-73 (Niach/exponential#2939). The two tests are not filed (#EDE-69).

### L-57 A fake can fail, stall and refuse

- Mistake: the fake backend never failed, the fake shell drew every document, the fake services answered at once, the stub database ignored the where-clause. The paths that broke were the ones no fake could reach.
- Why: fakes were written for the happy path they made fast.
- Rule: each fake can inject a failure per step, and a contract is tested against the real consumer's behaviour.
- eDE: per defect, #EXP-1895 (Niach/exponential#2937), #EDE-110 (Niach/exponential#3002), #EXP-1935.

### L-58 A test does not compare today against a fixed date

- Mistake: a test dated its data relative to now against a fixed cutoff. It turned red on the release PR and stopped the cut of eDE 0.2.28 on 2026-11-17.
- Why: the review state gives no cause; such a test passes on the day it is written.
- Rule: a test gets its clock as an input.
- eDE: source #EDE-69 (the release note in the review state); the fix is outside this review.

### L-59 A golden capture waits for the frame it asserts

- Mistake: a golden capture raced the first draw (the start menu was missing once and the picture passed on a rerun), and the golden gate tolerated 0.5 % of the pixels while its comments said a changed pixel fails.
- Why: capture and render were not tied to each other, and the tolerance was a shared default.
- Rule: a capture is taken after the frame that contains the state under test, and a tolerance is the measured noise, stated next to the test.
- eDE: filed, #EDE-106. The tolerance is not filed (#EDE-69). Rerusted already compares what the compositor draws exactly (decisions.md, #RUST-21).

### L-60 A release is run in a real session first

- Mistake: four runs on a real session in a VM found what no headless test saw: the start menu without the keyboard (#EDE-86), the polkit prompt (#EDE-87), the first X11 window without keys (#EDE-88), the taskbar preview that closed the start menu (#EDE-89). Three of the four were already in the released 0.2.6.
- Why: the headless tests run no login session, no polkit of the distro and no real keyboard.
- Rule: before a release, the build is installed in a VM, driven with keys as scancodes, and checked against a fixed list (lock, prompts, focus, the journal); the installed build is proved by hash, not by version string.
- eDE: done by hand on 2026-11-17 (vm-run.md, compare.md, recheck.md, polkit-check.md on #EDE-69); fixes Niach/exponential#2974, Niach/exponential#2975, Niach/exponential#2981, Niach/exponential#2984. Rerusted has the rig for it (tier 3, #RUST-21).

## Error reports

### L-61 Every binary and every child reports, and a dead feature is an error

- Mistake: three of eDE's seven binaries and neither worker child had error reporting. Push to talk panicked deterministically from 2026-11-09 and no report exists. Voice and computer use logged every dead end as a warning, and only errors become reports.
- Why: each `main()` copied its own set-up, a worker branch above it opted out silently, and an empty error project looked like health.
- Rule: one process entry that every binary and re-exec mode goes through, checked by a test that lists the binaries; a child hands its exit status to its parent as data; feature health is a typed state whose step into "dead" is what reports.
- eDE: not filed (#EDE-69); the panic itself is fixed, #EXP-1901 (Niach/exponential#2941).

## Still open

What this document cannot say yet. A second PR appends it.

- The background core: the seat filter on eDE's Smithay revision, the second Desk, the tools against a space (steps F1, F2 and F4 of the plan). Whether L-38 to L-41 hold up, and what the port from Smithay 0.7.0 to 0.8 costs.
- The acceptance test of L-39: it does not exist in eDE yet.
- The real runs of phase 3 (#EDE-69): a real call on eDE and from an Android phone, push to talk on a real build, a per-run grant against a real compositor, a GPU client in a capture on real hardware, two monitors while locked.
- The decisions not taken: a prompt over a fullscreen window (#EDE-107), keyboard selection in the start menu (#EDE-92), call content through the relay (L-50).
- The Voice fixes still in work (#EXP-1930, #EXP-1934, #EXP-1935, #EXP-1936, #EXP-1937): the rules L-44 to L-51 are written from the review, not from a landed fix.
