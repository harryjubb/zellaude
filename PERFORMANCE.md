# Zellaude Performance Assessment & Improvement Plan

## Executive Summary

Zellaude has several compounding performance issues that degrade over time and
worsen on session resume. The root causes fall into three categories: excessive
process spawning in the hook script, unnecessary re-renders in the WASM plugin,
and an N-squared inter-instance sync protocol. Together, a 10-tab session with
3 active Claude sessions can produce ~30 jq processes/second and ~70
renders/second across all plugin instances during moderate tool use.

---

## Issues by Severity

### P0 — Hook script spawns 5 jq processes per event

**Location:** `scripts/zellaude-hook.sh:16-40`

**Problem:** Every Claude Code hook event (PreToolUse, PostToolUse,
UserPromptSubmit, etc.) invokes the hook script, which spawns 4 separate `jq`
processes to extract fields from stdin, plus a 5th to build the output payload:

```bash
HOOK_EVENT=$(echo "$INPUT" | jq -r '.hook_event_name // empty')
SESSION_ID=$(echo "$INPUT" | jq -r '.session_id // empty')
TOOL_NAME=$(echo "$INPUT" | jq -r '.tool_name // empty')
CWD=$(echo "$INPUT" | jq -r '.cwd // empty')
# ... then a 5th jq to build PAYLOAD
```

Each tool call generates at least 2 events (Pre + Post) = 10 jq processes per
tool call per Claude session. With 3 active sessions doing ~2 tool calls/sec,
that's **~60 jq spawns/second** sustained. Over a day this is millions of
short-lived processes, creating CPU pressure, scheduler contention, and degraded
system responsiveness.

PermissionRequest events are even worse: they add a 6th `jq` call to read
settings, plus an `osascript` call (~100ms) to check the frontmost app.

**Fix:** Single jq invocation to extract all fields and build the payload in one
pass. This cuts process spawning by ~80%.

```bash
INPUT=$(cat)
PAYLOAD=$(echo "$INPUT" | jq -c \
  --arg pane_id "$ZELLIJ_PANE_ID" \
  --arg zellij_session "$ZELLIJ_SESSION_NAME" \
  --arg term_program "${TERM_PROGRAM:-}" \
  '{
    pane_id: ($pane_id | tonumber),
    session_id: .session_id,
    hook_event: .hook_event_name,
    tool_name: .tool_name,
    cwd: .cwd,
    zellij_session: $zellij_session,
    term_program: (if $term_program == "" then null else $term_program end)
  }')
HOOK_EVENT=$(echo "$PAYLOAD" | jq -r '.hook_event')
```

This is 2 jq calls instead of 5 (the second is needed for the HOOK_EVENT
variable used in the notification branch). Or even 1 if we restructure the
notification logic to check hook_event inside the jq expression.

**Effort:** Low
**Impact:** High — largest single contributor to system-wide slowdown

---

### P0 — Unconditional 1 Hz re-render across all instances

**Location:** `src/main.rs:146-155`

**Problem:** The timer fires every second and returns `true` (triggering a
render) whenever `has_elapsed_display()` is true. This function returns true
when any session is non-idle and older than 30 seconds — which is nearly always
the case when Claude sessions are running.

```rust
has_flashes || stale_changed || flash_changed || self.has_elapsed_display()
```

With N tabs, there are N plugin instances, each re-rendering every second, even
when nothing visible has changed. Each render builds a full ANSI string, scans
all sessions multiple times, and writes+flushes to stdout.

**Fix:** Cache the previously rendered output string. On timer tick, build the
new string but compare against the cache — only `print!` + `flush()` if it
actually changed. Elapsed time text changes at most once per second, and only
for the instance on the active tab (others aren't visible), so most renders
become no-ops.

```rust
// In State:
pub last_rendered: String,

// In render:
if buf == state.last_rendered {
    return; // nothing changed, skip I/O
}
state.last_rendered = buf.clone();
print!("{buf}");
```

**Effort:** Low
**Impact:** High — eliminates N-1 redundant renders per second at steady state

---

### P1 — N-squared sync storm on session resume

**Location:** `src/main.rs:157-172`, `src/main.rs:313-321`

**Problem:** When a Zellij session resumes, all N plugin instances reload
simultaneously. Each calls `request_sync()` which sends `zellaude:request` to
all N instances. Each receiver responds with `broadcast_sessions()` which sends
`zellaude:sync` to all N instances.

Message flow: N requests x N broadcast responses x N receives = **N^3 message
processing events**. For 10 tabs = 1,000 events; for 20 tabs = 8,000. Each sync
message triggers JSON deserialization + merge + re-render.

On a fresh resume, all session maps are empty (WASM state is lost), so the
entire storm is pure overhead — broadcasting and merging empty maps.

**Fix options (pick one):**

A. **Debounce sync requests.** On `PermissionRequestResult`, set a flag and
   delay the actual `request_sync()` by 2-3 seconds via the timer. Only the
   first instance to complete the delay sends the request. Other instances
   will have already received state from the first broadcast.

B. **Skip sync when empty.** In the `zellaude:request` handler, don't call
   `broadcast_sessions()` if `self.sessions.is_empty()`.

C. **Elect a coordinator.** Use a deterministic rule (e.g., lowest plugin
   instance ID, or first to respond) so only one instance broadcasts.

Option B is simplest and handles the resume case. Option A is more robust.

**Effort:** Low (option B) / Medium (option A)
**Impact:** High on resume, none at steady state

---

### P1 — Every pipe message triggers renders in ALL instances

**Location:** `src/main.rs:180-192`

**Problem:** The `"zellaude"` pipe handler always returns `true`, which triggers
a render in every plugin instance that receives the message. Since `zellij pipe`
delivers to all instances, one hook event → N renders. Most of these are for
instances on inactive tabs where the user can't even see the output.

Combined with P0 (jq spawning), a single tool call produces:
- 2 hook events x N instances = 2N pipe handler calls = 2N renders

**Fix:** Only return `true` if the event actually changed this instance's
visible state. Compare the session's previous activity to the new activity — if
unchanged, return `false`. At minimum, the Notification event handler in
`event_handler.rs:29-34` should not trigger a render (it only updates a
timestamp, which is only visible through elapsed time on the next timer tick
anyway).

**Effort:** Low
**Impact:** Medium — reduces renders proportional to event rate x tab count

---

### P2 — Redundant O(T*S) session scans in render

**Location:** `src/render.rs:244-253, 314-324, 394-398`

**Problem:** `render_tabs()` scans all sessions multiple times per tab:

1. **Lines 244-253:** For each tab, scan all sessions to find the highest-
   priority session → O(T*S)
2. **Lines 314-324:** For each tab, scan all sessions again to check for
   active flashes → O(T*S)
3. **Lines 394-398:** For each tab, scan all sessions again to find a waiting
   session for the click region → O(T*S)

Total: 3 x O(T*S) per render. With T=20 tabs and S=20 sessions, that's 1,200
iterations per render, per instance, up to once per second.

**Fix:** Pre-compute a `HashMap<tab_index, Vec<&SessionInfo>>` grouping
sessions by tab before the render loop. Then each lookup is O(sessions_in_tab)
instead of O(all_sessions). This also makes the flash and waiting checks
per-tab instead of full-scan.

```rust
let sessions_by_tab: HashMap<usize, Vec<&SessionInfo>> = ...;
// Then in the loop:
let tab_sessions = sessions_by_tab.get(&tab.position).unwrap_or(&empty);
let best = tab_sessions.iter().max_by_key(|s| activity_priority(&s.activity));
```

**Effort:** Low
**Impact:** Low at current scale, but prevents O(n^2) growth

---

### P2 — Heap allocations in render hot path

**Location:** `src/render.rs:54-60`

**Problem:** `fg()` and `bg()` return heap-allocated `String`s and are called
many times per render (every tab, every color transition). Each call does a
`format!()` → heap allocation.

```rust
fn fg(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}
```

**Fix:** Write ANSI codes directly into the buffer using `write!()` instead of
allocating intermediate strings:

```rust
fn write_fg(buf: &mut String, r: u8, g: u8, b: u8) {
    let _ = write!(buf, "\x1b[38;2;{r};{g};{b}m");
}
```

Or pre-compute color strings for the fixed palette (BAR_BG, TAB_BG_ACTIVE,
etc.) as `const` or `lazy_static`.

**Effort:** Medium (touches many call sites)
**Impact:** Low — reduces allocation pressure but unlikely to be user-visible

---

### P2 — Installer race condition can duplicate hooks

**Location:** `src/installer.rs:18-77`

**Problem:** On `PermissionRequestResult`, all N instances call `run_install()`
concurrently. The install script has a version check that exits early if
current, but if it falls through (on version change), the two-step jq operation
(remove old hooks, then add new hooks) can race:

1. Instance A reads settings, removes zellaude hooks, writes
2. Instance B reads settings (after A's remove but before A's add), removes
   (nothing to remove), writes
3. Instance A reads settings, adds hooks, writes
4. Instance B reads settings (now has A's hooks), adds hooks again, writes
5. Result: **double hook entries** — every event now fires the hook script twice

Each extra copy multiplies all hook-related overhead. This persists across
sessions (settings.json is on disk) and compounds on each version upgrade or
manual reinstall.

**Fix:** Add a file lock (`flock`) around the non-early-exit path of the
installer. Or better: do the remove+add in a single atomic jq expression.

```bash
jq --argjson events "$EVENTS" --argjson entry "$ENTRY" '
  # Remove old entries and add new in one pass
  .hooks //= {} |
  reduce ($events[]) as $event (.;
    .hooks[$event] = [
      ((.hooks[$event] // [])[] |
        select((.hooks // []) | all((.command // "") | endswith("zellaude-hook.sh") | not)))
    ] + $entry
  )
' "$SETTINGS" > "$tmp" && mv "$tmp" "$SETTINGS"
```

**Effort:** Low
**Impact:** Prevents catastrophic degradation on version changes

---

### P3 — No early exit for low-value events in hook script

**Location:** `scripts/zellaude-hook.sh`

**Problem:** The Notification event is handled in the plugin by just updating a
timestamp (event_handler.rs:29-34). But the hook script still reads stdin,
parses all fields, builds the full payload, and shells out to `zellij pipe`.
This event fires frequently during long-running sessions.

**Fix:** After extracting `HOOK_EVENT` (ideally in the single-jq-pass from the
P0 fix), exit early for events that don't need the full pipeline, or skip the
pipe entirely for Notification events since they only update a timestamp that
the timer will render anyway.

**Effort:** Low
**Impact:** Low-Medium — depends on Notification event frequency

---

## Improvement Plan

### Phase 1 — Quick wins (biggest impact, least effort)

These three changes address the core performance issues and can be done
independently:

1. **Consolidate hook script jq calls** (P0)
   - Rewrite `zellaude-hook.sh` to use 1-2 jq invocations instead of 5-6
   - Add early exit for Notification events
   - Expected: ~80% reduction in process spawning

2. **Add render output caching** (P0)
   - Store last rendered string in State
   - Skip print+flush when output hasn't changed
   - Expected: eliminates nearly all redundant renders

3. **Skip empty broadcasts on resume** (P1)
   - In `zellaude:request` handler, return early if sessions map is empty
   - Expected: eliminates N^3 resume storm entirely

### Phase 2 — Reduce render frequency

4. **Conditional render on pipe events** (P1)
   - Return `false` from the `"zellaude"` pipe handler when the event doesn't
     change visible state (wrong tab, same activity, Notification event)
   - Consider: only render if the affected session is on the active tab

5. **Pre-group sessions by tab for render** (P2)
   - Build a tab→sessions index before the render loop
   - Eliminates 3x O(T*S) full scans

### Phase 3 — Harden

6. **Atomic installer** (P2)
   - Merge the remove+add jq steps into a single expression
   - Prevents hook duplication on concurrent installs

7. **Reduce render allocations** (P2)
   - Write ANSI codes directly to buffer instead of via intermediate Strings
   - Pre-compute color strings for the fixed palette

### Phase 4 — Stretch goals

8. **Targeted pipe routing** — investigate whether Zellij supports sending pipe
   messages to a specific plugin instance rather than all instances. If so,
   designate one instance as the "primary" that receives hook events and
   broadcasts state changes only when needed.

9. **Hook event batching** — for rapid tool call sequences, the hook script
   could buffer events briefly and send a single batched pipe message. Adds
   complexity and latency; only worth it if Phase 1-2 are insufficient.

---

## Steady-State Load Model (before vs after Phase 1-2)

Assumptions: 10 tabs, 3 active Claude sessions, ~2 tool calls/sec each

| Metric | Before | After Phase 1 | After Phase 2 |
|--------|--------|---------------|---------------|
| jq processes/sec | ~60 | ~12 | ~12 |
| Renders/sec (all instances) | ~70 | ~10 | ~4 |
| Pipe messages/sec | ~12 | ~12 | ~12 |
| Resume storm messages | ~1000 | ~0 | ~0 |

Phase 1 alone should resolve the "unusable after a day" degradation. Phase 2
makes it sustainable long-term.
