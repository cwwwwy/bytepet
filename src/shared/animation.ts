/**
 * Sprite-animation math shared by the pet overlay window and the settings
 * preview. `frameIndex` is byte-for-byte the same logic the pet window uses, so
 * the preview never disagrees with the real overlay.
 */

import type { Animation, FrameSpec, PetEntry, PetState } from "./types";

/** The subset of [`Animation`] the frame math needs (keeps tests dependency-free). */
export interface AnimationLike {
  sprites: number[];
  durationsMs: number[];
  loopAnim: boolean;
}

export interface SourceRect {
  sx: number;
  sy: number;
  sw: number;
  sh: number;
}

/**
 * Index into `animation.sprites` for a given elapsed time.
 *
 * - looping tracks wrap around the summed duration;
 * - one-shot tracks hold their last frame once finished;
 * - empty/zero-duration tracks resolve to frame 0.
 */
export function frameIndex(animation: AnimationLike, elapsedMs: number): number {
  const total = animation.durationsMs.reduce((sum, ms) => sum + ms, 0);
  if (total <= 0 || animation.sprites.length === 0) return 0;
  let t = elapsedMs;
  if (animation.loopAnim) {
    t = elapsedMs % total;
  } else if (t >= total) {
    return animation.sprites.length - 1;
  }
  let acc = 0;
  for (let i = 0; i < animation.durationsMs.length; i++) {
    acc += animation.durationsMs[i];
    if (t < acc) return i;
  }
  return animation.sprites.length - 1;
}

/** Total duration of one pass over the animation track. */
export function totalDuration(animation: AnimationLike): number {
  return animation.durationsMs.reduce((sum, ms) => sum + ms, 0);
}

/** Sprite index drawn at `elapsedMs` (clamped to the sprite array). */
export function spriteAt(animation: AnimationLike, elapsedMs: number): number {
  if (animation.sprites.length === 0) return -1;
  const index = frameIndex(animation, elapsedMs);
  return animation.sprites[Math.min(index, animation.sprites.length - 1)];
}

/** Source rectangle of one sprite cell inside the spritesheet atlas. */
export function sourceRect(sprite: number, frame: FrameSpec): SourceRect {
  const columns = Math.max(1, frame.columns);
  return {
    sx: (sprite % columns) * frame.width,
    sy: Math.floor(sprite / columns) * frame.height,
    sw: frame.width,
    sh: frame.height,
  };
}

/** Best animation for a state, falling back to `idle` and then to any track. */
export function pickAnimation(
  pet: Pick<PetEntry, "animations">,
  state: PetState,
): Animation | null {
  const animations = pet.animations;
  if (!animations) return null;
  const direct = animations[state];
  if (direct) return direct;
  const idle = animations.idle;
  if (idle) return idle;
  const first = Object.values(animations).find((anim): anim is Animation => Boolean(anim));
  return first ?? null;
}

/** Every animation that can actually be drawn (has sprites). */
export function drawableAnimations(pet: Pick<PetEntry, "animations">): Animation[] {
  if (!pet.animations) return [];
  return Object.values(pet.animations).filter(
    (anim): anim is Animation => Boolean(anim) && anim.sprites.length > 0,
  );
}
