# BytePet

BytePet is being rebuilt as a lightweight, Rust-only desktop pet.

The new application keeps the parts that made the original Codex pet useful:

- reads Codex-compatible pet packages from `~/.codex/pets`
- renders the 8x9 / 8x11 spritesheet animation state machine
- supports a transparent, always-on-top pet window
- supports click, double-click, drag, right-click menu, gaze and a speech bubble
- can switch pets at runtime from `~/.codex/pets`, `~/.unipet/pets` or the local library
- exposes the local state protocol so Codex hooks can drive the animation
- stores a simplified persona and lightweight pet memory
- uses one DeepSeek API transport for short, intelligent greetings

The exact frame inventory of the shipped Codex pet and every replicated
behaviour is documented in [docs/PET_NATIVE.md](docs/PET_NATIVE.md).

The previous Tauri + WebView application is kept under `legacy/` as a reference
and is no longer part of the workspace.

## Workspace

```text
crates/bytepet-core/   Pet format, animation engine, persona, memory, DeepSeek client
crates/bytepet-app/    egui/eframe desktop application
legacy/                Previous Tauri app and frontend, reference only
```

## Build

```powershell
cargo run -p bytepet-app
```

The Windows MSVC target still requires the MSVC linker. Install Visual Studio
Build Tools with the "Desktop development with C++" workload before building.

## Interactions

| Input | Behaviour |
|---|---|
| Left click | wave + a greeting bubble (DeepSeek when a key is configured, else the persona's fallback greeting) |
| Double click | jump |
| Drag | move the pet |
| Right click | menu: open settings / close pet |
| Cursor at either side | the V2 look row plays once (the pet glances that way) |
| Tray icon | open settings / show-hide pet / quit |
| Every 45 min | activity reminder: the pet walks a short distance and asks you to stand up |

## Inspecting a pet

```powershell
cargo run -p bytepet-core --example pet_inspect -- "$env:USERPROFILE\.codex\pets\boba" .scratch\boba
```

Prints the resolved grid, how many frames each row actually draws and the
animation table the engine builds; the optional output directory receives one
PNG per animation row.

## State protocol

BytePet listens on `127.0.0.1:17872` (settings -> 状态协议) so hooks and scripts
can drive the pet:

```bash
curl -XPOST http://127.0.0.1:17872/state \
  -H 'content-type: application/json' \
  -d '{"source":"codex","state":"running","message":"正在跑测试","ttlMs":120000}'
```

`GET /health` returns the current pet/persona/state snapshot and `GET /pets`
lists the discovered pets.

## DeepSeek

The first version uses the OpenAI-compatible Chat Completions endpoint:

```text
base URL: https://api.deepseek.com/v1
model:    deepseek-v4-flash
```

Set the API key in the environment:

```powershell
$env:DEEPSEEK_API_KEY = "sk-..."
cargo run -p bytepet-app
```

The settings window can also save the key to the operating system keychain.
Greeting requests are non-streaming and use a small token budget; if the API is
unavailable, BytePet falls back to the persona's fixed or time-based greeting.

## Memory

Memory is stored in `memory.json` under the platform config directory. It is
not a chat transcript. It contains only:

- long-term facts
- recent interaction events
- last-seen and last-greeting state

This keeps the rebuilt application small while still allowing greetings to
refer to stable preferences and recent context.
