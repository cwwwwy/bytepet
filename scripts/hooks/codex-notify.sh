#!/usr/bin/env bash
# ---------------------------------------------------------------------------
# REFERENCE TEMPLATE — not the runtime script.
#
# `bytepet hooks install codex` generates its own copy at
#   <data_dir>/hooks/codex-notify.sh
# with `__PET_PORT__` replaced by the real port and `__PET_CHAIN__` replaced by
# the `notify` argv that was configured before pet was installed. The generated
# file is chmod 755 and is what `~/.codex/config.toml` points at:
#
#   notify = ["<data_dir>/hooks/codex-notify.sh"]
#
# Codex calls `notify` with a single JSON argument (stdin is not used), e.g.
#   {"type":"agent-turn-complete","thread-id":"…","turn-id":"…","cwd":"…",
#    "input-messages":[…],"last-assistant-message":"…"}
#
# The wrapper forwards a translated AgentEvent to the pet server, chains to the
# previous notify command, and always exits 0 — even when the pet app is off.
# ---------------------------------------------------------------------------
PET_PORT=__PET_PORT__
PET_SOURCE='codex'
PET_STATE_URL="http://127.0.0.1:${PET_PORT}/state"

# Previously configured `notify` argv (empty when notify was unset).
PET_CHAIN=(
__PET_CHAIN__
)

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

# Codex passes the notification JSON as the last argument; some builds pipe it
# on stdin, so accept both.
pet_payload=''
if [ ! -t 0 ]; then pet_payload="$(cat 2>/dev/null || true)"; fi
if [ -z "$pet_payload" ] && [ "$#" -gt 0 ]; then
  for pet_last in "$@"; do :; done
  pet_payload="$pet_last"
fi

pet_state=''
if [ -n "$pet_payload" ]; then
  pet_event="$(pet_json_field "$pet_payload" type)"
  pet_event="$(printf '%s' "$pet_event" | tr '[:upper:]' '[:lower:]')"
  case "$pet_event" in
    *session-start*|*session_start*|*sessionstart*) pet_state='waving' ;;
    *turn-complete*|*turn_completed*|*turn-ended*|*turn_end*|*complete*|*done*|*stop*|*session-end*|*session_end*) pet_state='review' ;;
    *turn-start*|*turn_start*|*turn-begin*|*started*|*start*) pet_state='running' ;;
    *prompt*|*submit*) pet_state='running' ;;
    *tool-use*|*tool_use*) pet_state='running' ;;
    *notification*|*notify*|*approval*|*permission*|*waiting*) pet_state='waiting' ;;
    *error*|*fail*) pet_state='failed' ;;
  esac
fi

if [ -n "$pet_state" ]; then
  pet_message="$(pet_json_field "$pet_payload" last-assistant-message 200)"
  [ -n "$pet_message" ] || pet_message="$(pet_json_field "$pet_payload" message 200)"
  pet_post_state "$pet_state" "$pet_message"
fi

# Chain to the notify command that was configured before pet.
if [ "${#PET_CHAIN[@]}" -gt 0 ]; then
  "${PET_CHAIN[@]}" "$@" >/dev/null 2>&1 || true
fi
exit 0
