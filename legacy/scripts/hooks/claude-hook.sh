#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# REFERENCE TEMPLATE — not the runtime script.
#
# `bytepet hooks install claude-code` generates its own copy at
#   <data_dir>/hooks/claude-hook.sh
# with `__PET_PORT__` replaced by the real port, and registers it as a command
# hook for SessionStart, UserPromptSubmit, PreToolUse, PostToolUse,
# Notification, Stop and SubagentStop in ~/.claude/settings.json:
#
#   "hooks": {
#     "Stop": [
#       { "hooks": [ { "type": "command",
#                      "command": "<data_dir>/hooks/claude-hook.sh" } ] }
#     ]
#   }
#
# Claude Code sends the hook payload as JSON on stdin. The wrapper maps
# `hook_event_name` to a bytepet state, POSTs it, prints nothing and always exits
# 0 so it can never block or steer Claude Code.
# ---------------------------------------------------------------------------
PET_PORT=__PET_PORT__
PET_SOURCE='claude-code'
PET_STATE_URL="http://127.0.0.1:${PET_PORT}/state"

pet_json_field() {
  # $1 = JSON text, $2 = key, $3 = max decoded chars (0 = unlimited).
  # Prints the raw (still JSON-escaped) string content, or nothing when absent.
  # awk instead of sed so escaped quotes survive and EOF does not drop output.
  printf '%s' "$1" | awk -v key="$2" -v maxlen="${3:-0}" '
    BEGIN { pattern = "\"" key "\"[[:space:]]*:[[:space:]]*\"" }
    {
      if (found) next
      idx = match($0, pattern)
      if (idx == 0) next
      rest = substr($0, idx + RLENGTH)
      out = ""; count = 0; i = 1
      while (i <= length(rest)) {
        c = substr(rest, i, 1)
        if (c == "\\") {
          esc = substr(rest, i, 2)
          if (maxlen > 0 && count >= maxlen) break
          out = out esc; count += 1; i += 2; continue
        }
        if (c == "\"") { print out; found = 1; exit }
        if (maxlen > 0 && count >= maxlen) break
        out = out c; count += 1; i += 1
      }
      print out; found = 1; exit
    }'
}

pet_post_state() {
  # $1 = state, $2 = message already JSON-escaped (may be empty).
  # Never fails and never blocks for long.
  pet_state="$1"
  pet_message="$2"
  if [ -z "$pet_state" ]; then return 0; fi
  pet_body="{\"source\":\"${PET_SOURCE}\",\"state\":\"${pet_state}\""
  if [ -n "$pet_message" ]; then
    pet_body="${pet_body},\"message\":\"${pet_message}\""
  fi
  pet_body="${pet_body}}"
  if command -v curl >/dev/null 2>&1; then
    curl -sS -m 2 -o /dev/null -X POST -H 'Content-Type: application/json' --data-binary "$pet_body" "$PET_STATE_URL" >/dev/null 2>&1 || true
  elif command -v wget >/dev/null 2>&1; then
    wget -q -T 2 -O /dev/null --header='Content-Type: application/json' --post-data="$pet_body" "$PET_STATE_URL" >/dev/null 2>&1 || true
  fi
  return 0
}

# Claude Code sends the hook payload as JSON on stdin.
pet_payload="$(cat 2>/dev/null || true)"

pet_state=''
pet_event="$(pet_json_field "$pet_payload" hook_event_name)"
case "$pet_event" in
  SessionStart) pet_state='waving' ;;
  UserPromptSubmit) pet_state='running' ;;
  PreToolUse|PostToolUse) pet_state='running' ;;
  Notification) pet_state='waiting' ;;
  Stop|SubagentStop|SessionEnd) pet_state='review' ;;
  *) pet_state='' ;;
esac

if [ -n "$pet_state" ]; then
  pet_message="$(pet_json_field "$pet_payload" message 200)"
  [ -n "$pet_message" ] || pet_message="$(pet_json_field "$pet_payload" last_assistant_message 200)"
  pet_post_state "$pet_state" "$pet_message"
fi
exit 0
