/**
 * Pet overlay renderer: draws the active spritesheet cell-by-cell with the
 * per-frame durations resolved by the Rust state machine.
 *
 * Rust owns all state; this file only draws and reports pointer intent.
 */

import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface FrameSpec {
  width: number;
  height: number;
  columns: number;
  rows: number;
}

interface Animation {
  state: string;
  row: number;
  sprites: number[];
  durationsMs: number[];
  loopAnim: boolean;
  fallback: string;
  totalMs: number;
}

interface PetEntry {
  id: string;
  displayName: string;
  spritesheet: string;
  frame: FrameSpec;
  animations: Record<string, Animation>;
}

interface BootstrapState {
  config: { pet: { scale: number; clickThrough: string } };
  pets: PetEntry[];
  activePet: PetEntry | null;
  currentState: PetStateEvent | null;
}

interface PetStateEvent {
  state: string;
  source: string;
  message: string | null;
  oneShot: boolean;
}

const canvas = document.getElementById("pet-canvas") as HTMLCanvasElement;
const ctx = canvas.getContext("2d")!;
const bubble = document.getElementById("bubble") as HTMLDivElement;

let pet: PetEntry | null = null;
let image: HTMLImageElement | null = null;
let animation: Animation | null = null;
let startedAt = performance.now();
let lastSprite = -1;
let oneShotReported = false;

function pickAnimation(pet: PetEntry, state: string): Animation | null {
  return pet.animations[state] ?? pet.animations.idle ?? null;
}

function play(state: string, message: string | null, oneShot: boolean) {
  if (!pet) return;
  const next = pickAnimation(pet, state);
  if (!next) return;
  if (animation?.state !== next.state) {
    animation = next;
    startedAt = performance.now();
    oneShotReported = false;
  }
  if (message) {
    bubble.textContent = message;
    bubble.classList.add("visible");
  } else {
    bubble.classList.remove("visible");
  }
  if (!oneShot) oneShotReported = false;
}

function frameIndex(anim: Animation, elapsed: number): number {
  const total = anim.durationsMs.reduce((a, b) => a + b, 0);
  if (total <= 0 || anim.sprites.length === 0) return 0;
  let t = elapsed;
  if (anim.loopAnim) {
    t = elapsed % total;
  } else if (t >= total) {
    return anim.sprites.length - 1;
  }
  let acc = 0;
  for (let i = 0; i < anim.durationsMs.length; i++) {
    acc += anim.durationsMs[i];
    if (t < acc) return i;
  }
  return anim.sprites.length - 1;
}

function resize() {
  if (!pet) return;
  const dpr = window.devicePixelRatio || 1;
  // The Rust side sizes the window to cell size * user scale, so fill it.
  const cssW = Math.max(1, window.innerWidth);
  const cssH = Math.max(1, window.innerHeight);
  canvas.style.width = `${cssW}px`;
  canvas.style.height = `${cssH}px`;
  canvas.width = Math.round(cssW * dpr);
  canvas.height = Math.round(cssH * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.imageSmoothingEnabled = true;
  ctx.imageSmoothingQuality = "high";
}

function loop() {
  requestAnimationFrame(loop);
  if (!pet || !image || !animation) return;

  const elapsed = performance.now() - startedAt;
  const index = frameIndex(animation, elapsed);
  const sprite = animation.sprites[index];
  const { width, height, columns } = pet.frame;
  const sx = (sprite % columns) * width;
  const sy = Math.floor(sprite / columns) * height;

  const destW = Math.max(1, window.innerWidth);
  const destH = Math.max(1, window.innerHeight);
  ctx.clearRect(0, 0, destW, destH);
  ctx.drawImage(image, sx, sy, width, height, 0, 0, destW, destH);

  if (sprite !== lastSprite) {
    lastSprite = sprite;
    void invoke("pet_sprite_index", { index: sprite }).catch(() => {});
  }
  if (!animation.loopAnim && !oneShotReported && elapsed >= animation.totalMs) {
    oneShotReported = true;
    void invoke("pet_animation_finished").catch(() => {});
  }
}

async function loadPet(entry: PetEntry | null) {
  pet = entry;
  image = null;
  animation = null;
  if (!pet) return;
  resize();

  const sources = [convertFileSrc(pet.spritesheet)];
  for (const src of sources) {
    const img = new Image();
    img.src = src;
    try {
      await img.decode();
      image = img;
      break;
    } catch (err) {
      console.warn("spritesheet load failed, trying fallback", err);
    }
  }

  if (!image) {
    // Asset protocol unavailable: pull the bytes through IPC instead.
    try {
      const dataUrl = await invoke<string>("pet_spritesheet_data_url", { id: pet.id });
      const img = new Image();
      img.src = dataUrl;
      await img.decode();
      image = img;
    } catch (err) {
      console.error("failed to decode spritesheet", err);
      return;
    }
  }
  play("idle", null, false);
}

async function boot() {
  const state = await invoke<BootstrapState>("get_bootstrap_state");
  await loadPet(state.activePet);
  // Listeners must exist before we tell Rust we are ready, otherwise the
  // greeting event could be lost.
  await listen<PetStateEvent>("pet://state", (event) => {
    play(event.payload.state, event.payload.message, event.payload.oneShot);
  });
  await listen<string>("pet://library-changed", async () => {
    const next = await invoke<BootstrapState>("get_bootstrap_state");
    await loadPet(next.activePet);
    if (next.currentState) {
      play(next.currentState.state, next.currentState.message, next.currentState.oneShot);
    }
  });
  window.addEventListener("resize", resize);
  loop();
  if (state.currentState) {
    play(state.currentState.state, state.currentState.message, state.currentState.oneShot);
  }
  await invoke("pet_ready").catch(() => {});
}

// --- pointer handling -------------------------------------------------------

let pressed = false;
let moved = false;
let pressAt = { x: 0, y: 0 };
let lastHover = 0;

// Codex row 4 is the "hover" creative action; trigger it at most once every 20s.
canvas.addEventListener("pointerenter", () => {
  const now = performance.now();
  if (now - lastHover < 20_000) return;
  lastHover = now;
  void invoke("set_pet_state", { state: "jumping" }).catch(() => {});
});

canvas.addEventListener("pointerdown", async (event) => {
  if (event.button !== 0) return;
  pressed = true;
  moved = false;
  pressAt = { x: event.clientX, y: event.clientY };
  document.body.classList.add("dragging");
  void invoke("pet_drag_started").catch(() => {});
  try {
    await getCurrentWindow().startDragging();
  } catch {
    /* ignore */
  }
});

window.addEventListener("pointerup", async (event) => {
  if (!pressed) return;
  pressed = false;
  document.body.classList.remove("dragging");
  const dist = Math.hypot(event.clientX - pressAt.x, event.clientY - pressAt.y);
  moved = dist > 4;
  await invoke("pet_drag_ended").catch(() => {});
  if (!moved) {
    await invoke("open_chat").catch(() => {});
  }
});

void boot().catch((err) => console.error("pet boot failed", err));
