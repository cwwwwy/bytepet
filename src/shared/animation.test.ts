/** Frame-index math shared with the pet overlay window. */

import { describe, expect, it } from "vitest";
import { drawableAnimations, frameIndex, pickAnimation, sourceRect, spriteAt, totalDuration } from "./animation";
import type { Animation, FrameSpec, PetEntry, PetState } from "./types";

function anim(partial: Partial<Animation> & Pick<Animation, "sprites" | "durationsMs" | "loopAnim">): Animation {
  return {
    state: "idle",
    row: 0,
    fallback: "idle",
    totalMs: partial.durationsMs.reduce((sum, ms) => sum + ms, 0),
    ...partial,
  };
}

const looping = anim({ sprites: [0, 1, 2, 3], durationsMs: [100, 100, 100, 100], loopAnim: true });
const oneShot = anim({ sprites: [10, 11, 12], durationsMs: [50, 50, 50], loopAnim: false });

describe("frameIndex", () => {
  it("walks a looping track frame by frame", () => {
    expect(frameIndex(looping, 0)).toBe(0);
    expect(frameIndex(looping, 99)).toBe(0);
    expect(frameIndex(looping, 100)).toBe(1);
    expect(frameIndex(looping, 250)).toBe(2);
    expect(frameIndex(looping, 399)).toBe(3);
  });

  it("wraps a looping track", () => {
    expect(frameIndex(looping, 400)).toBe(0);
    expect(frameIndex(looping, 450)).toBe(0);
    expect(frameIndex(looping, 500)).toBe(1);
    expect(frameIndex(looping, 1234)).toBe(0);
  });

  it("holds the last frame of a one-shot track", () => {
    expect(frameIndex(oneShot, 0)).toBe(0);
    expect(frameIndex(oneShot, 50)).toBe(1);
    expect(frameIndex(oneShot, 100)).toBe(2);
    expect(frameIndex(oneShot, 149)).toBe(2);
    expect(frameIndex(oneShot, 150)).toBe(2);
    expect(frameIndex(oneShot, 99999)).toBe(2);
  });

  it("clamps out-of-range indices to the sprite array", () => {
    const short = anim({ sprites: [5, 6], durationsMs: [10, 10, 10, 10], loopAnim: false });
    expect(frameIndex(short, 35)).toBe(3);
    expect(spriteAt(short, 35)).toBe(6);
    expect(spriteAt(short, 0)).toBe(5);
  });

  it("degrades gracefully for empty or zero-duration tracks", () => {
    expect(frameIndex(anim({ sprites: [], durationsMs: [100], loopAnim: true }), 50)).toBe(0);
    expect(frameIndex(anim({ sprites: [1, 2], durationsMs: [0, 0], loopAnim: true }), 50)).toBe(0);
    expect(spriteAt(anim({ sprites: [], durationsMs: [1], loopAnim: true }), 0)).toBe(-1);
  });

  it("computes the total duration", () => {
    expect(totalDuration(looping)).toBe(400);
    expect(totalDuration(oneShot)).toBe(150);
  });
});

describe("sourceRect", () => {
  const frame: FrameSpec = { width: 24, height: 32, columns: 8, rows: 4 };
  it("maps a sprite index onto atlas coordinates", () => {
    expect(sourceRect(0, frame)).toEqual({ sx: 0, sy: 0, sw: 24, sh: 32 });
    expect(sourceRect(7, frame)).toEqual({ sx: 168, sy: 0, sw: 24, sh: 32 });
    expect(sourceRect(8, frame)).toEqual({ sx: 0, sy: 32, sw: 24, sh: 32 });
    expect(sourceRect(9, frame)).toEqual({ sx: 24, sy: 32, sw: 24, sh: 32 });
  });
});

describe("pickAnimation", () => {
  const pet = {
    animations: {
      idle: anim({ sprites: [0], durationsMs: [100], loopAnim: true }),
      waving: anim({ state: "waving", sprites: [4], durationsMs: [100], loopAnim: false }),
    },
  } as unknown as Pick<PetEntry, "animations">;

  it("prefers the requested state", () => {
    expect(pickAnimation(pet, "waving" as PetState)?.state).toBe("waving");
  });

  it("falls back to idle and then to any track", () => {
    expect(pickAnimation(pet, "jumping" as PetState)?.state).toBe("idle");
    const onlyOther = { animations: { waving: pet.animations.waving } } as unknown as Pick<
      PetEntry,
      "animations"
    >;
    expect(pickAnimation(onlyOther, "jumping" as PetState)?.state).toBe("waving");
  });

  it("returns null when there is nothing to draw", () => {
    expect(pickAnimation({ animations: {} } as unknown as Pick<PetEntry, "animations">, "idle")).toBeNull();
  });

  it("lists drawable animations only", () => {
    const mixed = {
      animations: {
        idle: pet.animations.idle,
        empty: anim({ sprites: [], durationsMs: [], loopAnim: false }),
      },
    } as unknown as Pick<PetEntry, "animations">;
    expect(drawableAnimations(mixed).map((item) => item.state)).toEqual(["idle"]);
  });
});
