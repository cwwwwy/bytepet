# BytePet

BytePet is being rebuilt as a lightweight, Rust-only desktop pet.

The new application keeps the parts that made the original Codex pet useful:

- reads Codex-compatible pet packages from `~/.codex/pets`
- renders the 8x9 / 8x11 spritesheet animation state machine
- supports a transparent, always-on-top pet window
- supports click, drag and a small speech bubble
- stores a simplified persona and lightweight pet memory
- uses one DeepSeek API transport for short, intelligent greetings

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
