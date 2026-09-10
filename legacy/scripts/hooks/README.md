# Agent hooks

Reference copies of the wrapper scripts that `bytepet hooks install` generates. The
CLI does **not** run these files: it writes its own copies under
`<data_dir>/hooks/` with the real port and the previous `notify` command baked
in. This directory exists so the behaviour can be reviewed without installing
anything.

## What gets installed where

| Agent       | Config file                    | What pet changes                                                                 | Wrapper (Unix / Windows)                                    |
| ----------- | ------------------------------ | -------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| Codex       | `~/.codex/config.toml`         | `notify = ["<data_dir>/hooks/codex-notify.sh"]` (chains to the previous `notify`) | `<data_dir>/hooks/codex-notify.sh` / `codex-notify.cmd`     |
| Claude Code | `~/.claude/settings.json`      | adds a `command` hook for 7 lifecycle events under `"hooks"`                      | `<data_dir>/hooks/claude-hook.sh` / `claude-hook.cmd`       |

`<data_dir>` is the app config directory (`com.bytepet.desktop` inside the platform
config dir, e.g. `~/Library/Application Support/com.bytepet.desktop` on macOS).

Both installers are non-destructive and reversible:

* the original config is backed up to `<config>.pet-backup`;
* an exact record (original `notify` argv / added events, wrapper path,
  timestamp, config fingerprint) is written to `<data_dir>/hooks/state.json`;
* `bytepet hooks uninstall` restores the backup byte-for-byte when the config was
  not edited after install, otherwise it removes only pet's entries;
* installing twice is a no-op, uninstalling twice is `Ok`.

## Codex

Codex invokes `notify` with a **single JSON argument** (not stdin), for example:

```json
{
  "type": "agent-turn-complete",
  "thread-id": "…",
  "turn-id": "…",
  "cwd": "/path/to/project",
  "input-messages": ["…"],
  "last-assistant-message": "Done."
}
```

`agent-turn-complete` is currently the only supported notify event; the wrapper
also understands `*-start*`, `*notification*`, `*approval*`, `*fail*` and
`*error*` shapes for forward compatibility. The payload is translated to an
`AgentEvent` (`source: "codex"`, mapped `state`, truncated `message`) and POSTed
to `http://127.0.0.1:<port>/state` with a 2 second timeout. Afterwards the
wrapper runs the `notify` argv that was configured before pet, passing the
original arguments through. It always exits 0, so a stopped pet app can never
break Codex notifications.

## Claude Code

Schema implemented against the official reference
(<https://code.claude.com/docs/en/hooks>):

```json
{
  "hooks": {
    "Stop": [
      {
        "matcher": "*",
        "hooks": [{ "type": "command", "command": "<data_dir>/hooks/claude-hook.sh" }]
      }
    ]
  }
}
```

Events installed: `SessionStart`, `UserPromptSubmit`, `PreToolUse`,
`PostToolUse`, `Notification`, `Stop`, `SubagentStop`. Events the installed CLI
is too old to support are skipped (the CLI version is probed with
`claude --version`; if it cannot be probed all events are installed). Legacy
configs that store plain `{"type":"command","command":"…"}` objects directly in
the event array are detected and preserved. The wrapper reads the JSON payload
from stdin, maps `hook_event_name` to a state, POSTs it, prints nothing and
exits 0.

## Removing the integration

```bash
bytepet hooks uninstall all          # or: bytepet hooks uninstall codex / claude-code
bytepet hooks status                 # verify
```

Manual removal, if you prefer:

1. Codex: restore `~/.codex/config.toml.pet-backup` over `~/.codex/config.toml`.
2. Claude Code: restore `~/.claude/settings.json.pet-backup`, or delete the
   handler entries whose `command` points at `<data_dir>/hooks/claude-hook.*`.
3. Delete `<data_dir>/hooks/` (wrappers and `state.json`).

## Template placeholders

The `.sh` / `.ps1` files here use two placeholders that the Rust installer
substitutes:

* `__PET_PORT__` — the port of the local status server (default `17872`).
* `__PET_CHAIN__` — one single-quoted argv element per line, the previous
  Codex `notify` command (empty when `notify` was unset).
