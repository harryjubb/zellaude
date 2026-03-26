# Zellaude Performance Improvements

## Context

Zellaude has compounding performance issues that degrade over time: excessive jq process spawning in the hook script (~60/sec), unnecessary re-renders across all plugin instances (~70/sec with 10 tabs), and N^3 sync storms on session resume. A 10-tab session with 3 active Claude sessions becomes noticeably sluggish. PERFORMANCE.md documents these issues thoroughly; this plan implements the fixes.

## Phase 1 — Quick Wins (independent, can be parallel)

### 1.1 Consolidate hook script jq calls (P0)
**File:** `scripts/zellaude-hook.sh`

Replace 5 separate jq invocations (lines 16-19 + 24-40) with 2:
1. Single jq call to build the PAYLOAD directly from stdin (combines field extraction + payload construction)
2. One jq call to extract HOOK_EVENT from the built payload

Add early exit for Notification events before the PermissionRequest notification block — Notification events still get piped but skip lines 43-128.

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
[ -z "$PAYLOAD" ] && exit 0
HOOK_EVENT=$(echo "$PAYLOAD" | jq -r '.hook_event // empty')
[ -z "$HOOK_EVENT" ] && exit 0

# Notification events skip desktop notification logic
if [ "$HOOK_EVENT" = "Notification" ]; then
  zellij pipe --name "zellaude" -- "$PAYLOAD"
  exit 0
fi
```

**Impact:** ~60% reduction in process spawning (5-6 jq → 2)

### 1.2 Add render output caching (P0)
**Files:** `src/state.rs`, `src/render.rs`

- Add `pub last_rendered: String` to `State` struct (state.rs:145)
- In `render_status_bar()` (render.rs:218-219), compare `buf` to `state.last_rendered` before print+flush
- Skip I/O if identical; update cache if different
- Click regions are still populated correctly (built during string construction, before the comparison)

```rust
// state.rs — add to State struct
pub last_rendered: String,

// render.rs — replace lines 218-219
if buf == state.last_rendered {
    return;
}
state.last_rendered = buf.clone();
print!("{buf}");
let _ = std::io::stdout().flush();
```

Note: also apply to the early return path (cols < 5, line 134-136).

**Impact:** Eliminates N-1 redundant renders per second at steady state

### 1.3 Skip empty broadcasts on resume (P1)
**File:** `src/main.rs:202-205`

```rust
"zellaude:request" => {
    if !self.sessions.is_empty() {
        self.broadcast_sessions();
    }
    false
}
```

**Impact:** Eliminates N^3 resume storm entirely (1000+ messages → 0 for 10 tabs)

## Phase 2 — Reduce Render Frequency

### 2.1 Conditional render returns from pipe handler (P1)
**Files:** `src/event_handler.rs`, `src/main.rs:190-191`

Change `handle_hook_event` to return `bool`. Return `false` when:
- Notification events (timestamp-only, timer handles display)
- SessionEnd for unknown pane (`sessions.remove()` returns `None`)
- Activity unchanged (consecutive Thinking events, etc.)

```rust
// event_handler.rs — new signature
pub fn handle_hook_event(state: &mut State, payload: HookPayload) -> bool

// main.rs:190-191 — use return value
event_handler::handle_hook_event(self, payload)
```

**Depends on:** 1.2 (render caching acts as safety net against missed renders)

### 2.2 Pre-group sessions by tab in render (P2)
**File:** `src/render.rs`

Build `HashMap<usize, Vec<&SessionInfo>>` at top of `render_tabs()`, replacing 3 separate O(T*S) scans (lines 244-253, 314-324, 394-398) with O(S) grouping + O(sessions_per_tab) lookups.

## Phase 3 — Hardening

### 3.1 Atomic installer (P2)
**File:** `src/installer.rs`

Merge the two-step jq (remove old hooks at lines 52-65, add new hooks at lines 70-74) into a single jq expression that removes old entries and adds new ones atomically. This eliminates the race where concurrent instances can read intermediate state and duplicate hook entries.

### 3.2 Reduce render heap allocations (P2)
**File:** `src/render.rs`

Replace `fg()`/`bg()` (lines 54-60) that return heap-allocated `String`s with direct `write!()` into the buffer. Inline the ANSI escape formatting at each call site (primarily `arrow()` and the tab rendering sections).

## Verification

1. `just lint` — clippy must pass with no warnings
2. `just build` — WASM must compile
3. `just install` — deploy locally and test in Zellij:
   - Open 5+ tabs, start 2-3 Claude sessions
   - Verify status bar renders correctly (activity icons, tab names, elapsed time)
   - Verify flash animation works on PermissionRequest
   - Verify click-to-focus works
   - Verify settings menu toggles work
   - Resume session and verify no visible delay
4. Manual hook script test: `echo '{"hook_event_name":"PreToolUse","session_id":"test","tool_name":"Bash","cwd":"/tmp"}' | ZELLIJ_SESSION_NAME=test ZELLIJ_PANE_ID=1 bash scripts/zellaude-hook.sh`

## Critical Files
- `scripts/zellaude-hook.sh` — hook script jq consolidation
- `src/state.rs` — add `last_rendered` field
- `src/render.rs` — render caching, session pre-grouping, allocation reduction
- `src/main.rs` — empty broadcast guard, conditional pipe returns
- `src/event_handler.rs` — return bool from handle_hook_event
- `src/installer.rs` — atomic jq expression
