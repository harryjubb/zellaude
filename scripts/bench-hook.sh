#!/usr/bin/env bash
# bench-hook.sh — Benchmark zellaude-hook.sh jq spawning overhead
#
# Measures end-to-end execution time of the hook script with stubbed
# zellij pipe (no actual Zellij needed). Reports average ms per invocation.
#
# Usage: just bench-hook
#        ./scripts/bench-hook.sh [iterations]

set -euo pipefail

ITERATIONS="${1:-100}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HOOK_SCRIPT="$SCRIPT_DIR/zellaude-hook.sh"

if [ ! -x "$HOOK_SCRIPT" ]; then
    echo "ERROR: $HOOK_SCRIPT not found or not executable" >&2
    exit 1
fi

# Create a temporary directory for the stub and settings
TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

# Stub zellij as a no-op (the hook calls `zellij pipe`)
STUB_ZELLIJ="$TMPDIR/zellij"
cat > "$STUB_ZELLIJ" << 'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$STUB_ZELLIJ"

# Stub osascript as a no-op (PermissionRequest path calls it)
STUB_OSASCRIPT="$TMPDIR/osascript"
cat > "$STUB_OSASCRIPT" << 'EOF'
#!/bin/sh
echo "SomeApp"
EOF
chmod +x "$STUB_OSASCRIPT"

# Stub terminal-notifier as a no-op
STUB_NOTIFIER="$TMPDIR/terminal-notifier"
cat > "$STUB_NOTIFIER" << 'EOF'
#!/bin/sh
exit 0
EOF
chmod +x "$STUB_NOTIFIER"

# Create a fake settings file for PermissionRequest path
SETTINGS_DIR="$TMPDIR/config/zellij/plugins"
mkdir -p "$SETTINGS_DIR"
echo '{"notifications":"Always","flash":"Once","elapsed_time":true}' > "$SETTINGS_DIR/zellaude.json"

# Prepend stubs to PATH so hook script finds them
export PATH="$TMPDIR:$PATH"
export ZELLIJ_SESSION_NAME="bench-session"
export ZELLIJ_PANE_ID="42"
export TERM_PROGRAM="iTerm.app"
export HOME="$TMPDIR"

# Sample payloads
PRETOOLUSE_PAYLOAD='{"hook_event_name":"PreToolUse","session_id":"sess-123","tool_name":"Bash","cwd":"/home/user/project"}'
POSTTOOLUSE_PAYLOAD='{"hook_event_name":"PostToolUse","session_id":"sess-123","tool_name":"Bash","cwd":"/home/user/project"}'
PERMISSION_PAYLOAD='{"hook_event_name":"PermissionRequest","session_id":"sess-123","tool_name":"Write","cwd":"/home/user/project"}'
NOTIFICATION_PAYLOAD='{"hook_event_name":"Notification","session_id":"sess-123","cwd":"/home/user/project"}'

run_bench() {
    local label="$1"
    local payload="$2"
    local n="$3"

    # Warm up (1 iteration)
    echo "$payload" | "$HOOK_SCRIPT" > /dev/null 2>&1 || true

    local start end elapsed_ms avg_ms
    start=$(python3 -c 'import time; print(time.time_ns())')

    for _ in $(seq 1 "$n"); do
        echo "$payload" | "$HOOK_SCRIPT" > /dev/null 2>&1 || true
    done

    end=$(python3 -c 'import time; print(time.time_ns())')
    elapsed_ms=$(python3 -c "print(f'{($end - $start) / 1_000_000:.1f}')")
    avg_ms=$(python3 -c "print(f'{($end - $start) / 1_000_000 / $n:.2f}')")

    printf "  %-24s %6s iterations  %8s ms total  %6s ms/call\n" "$label" "$n" "$elapsed_ms" "$avg_ms"
}

echo "Hook Script Benchmark (zellaude-hook.sh)"
echo "========================================="
echo ""
echo "Iterations per event type: $ITERATIONS"
echo ""

run_bench "PreToolUse" "$PRETOOLUSE_PAYLOAD" "$ITERATIONS"
run_bench "PostToolUse" "$POSTTOOLUSE_PAYLOAD" "$ITERATIONS"
run_bench "PermissionRequest" "$PERMISSION_PAYLOAD" "$ITERATIONS"
run_bench "Notification" "$NOTIFICATION_PAYLOAD" "$ITERATIONS"

echo ""
echo "Note: PreToolUse/PostToolUse use 2 jq processes each (consolidated from 5)."
echo "      PermissionRequest uses 2 jq + settings read + osascript."
echo "      Notification exits early after 2 jq + zellij pipe (skips notification logic)."

# If hyperfine is available, run a more rigorous benchmark
if command -v hyperfine >/dev/null 2>&1; then
    echo ""
    echo "--- hyperfine (more precise) ---"
    echo ""
    hyperfine \
        --warmup 3 \
        --min-runs "$ITERATIONS" \
        --export-markdown /dev/null \
        -n "PreToolUse" "echo '$PRETOOLUSE_PAYLOAD' | ZELLIJ_SESSION_NAME=bench ZELLIJ_PANE_ID=42 HOME=$TMPDIR PATH=$TMPDIR:\$PATH $HOOK_SCRIPT" \
        -n "PermissionRequest" "echo '$PERMISSION_PAYLOAD' | ZELLIJ_SESSION_NAME=bench ZELLIJ_PANE_ID=42 HOME=$TMPDIR PATH=$TMPDIR:\$PATH TERM_PROGRAM=iTerm.app $HOOK_SCRIPT" \
        2>&1 || echo "(hyperfine benchmark failed, see above)"
fi
