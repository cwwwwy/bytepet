/**
 * Live spritesheet preview used by the pet picker.
 *
 * Frame math is delegated to `shared/animation.ts` (the exact same code the pet
 * overlay runs), the atlas is loaded through `convertFileSrc`, and the rAF loop
 * is suspended whenever the card scrolls out of view or the window is hidden.
 */

import { useEffect, useMemo, useRef } from "preact/hooks";
import { convertFileSrc } from "@tauri-apps/api/core";
import { frameIndex, sourceRect } from "../../shared/animation";
import { pickAnimation } from "../../shared/animation";
import { t } from "../../shared/i18n";
import type { PetEntry } from "../../shared/types";

export interface PetPreviewProps {
  pet: PetEntry;
  /** Extra multiplier on top of the auto-fit scale. */
  scale?: number;
  class?: string;
}

export function PetPreview({ pet, scale = 1, class: className }: PetPreviewProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const stageRef = useRef<HTMLDivElement>(null);
  const animation = useMemo(() => pickAnimation(pet, "idle"), [pet]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const stage = stageRef.current;
    if (!canvas || !stage || !animation || animation.sprites.length === 0) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    let disposed = false;
    let frame = 0;
    let lastTimestamp = 0;
    let elapsed = 0;
    let visible = true;
    let image: HTMLImageElement | null = null;

    const applySize = (): void => {
      const dpr = window.devicePixelRatio || 1;
      const availableW = Math.max(24, stage.clientWidth - 16);
      const availableH = Math.max(24, stage.clientHeight - 16);
      const fit = Math.min(
        1,
        availableW / Math.max(1, pet.frame.width),
        availableH / Math.max(1, pet.frame.height),
      );
      const cssW = Math.max(1, Math.round(pet.frame.width * scale * fit));
      const cssH = Math.max(1, Math.round(pet.frame.height * scale * fit));
      canvas.style.width = `${cssW}px`;
      canvas.style.height = `${cssH}px`;
      canvas.width = Math.max(1, Math.round(cssW * dpr));
      canvas.height = Math.max(1, Math.round(cssH * dpr));
      ctx.setTransform(canvas.width / cssW, 0, 0, canvas.height / cssH, 0, 0);
      ctx.imageSmoothingEnabled = true;
    };

    const draw = (): void => {
      if (disposed || !image || !image.complete || image.naturalWidth === 0) return;
      const index = frameIndex(animation, elapsed);
      const sprite = animation.sprites[Math.min(index, animation.sprites.length - 1)];
      const rect = sourceRect(sprite, pet.frame);
      const cssW = canvas.clientWidth || pet.frame.width;
      const cssH = canvas.clientHeight || pet.frame.height;
      ctx.clearRect(0, 0, cssW, cssH);
      ctx.drawImage(image, rect.sx, rect.sy, rect.sw, rect.sh, 0, 0, cssW, cssH);
    };

    const tick = (timestamp: number): void => {
      if (disposed) return;
      if (lastTimestamp !== 0) elapsed += timestamp - lastTimestamp;
      lastTimestamp = timestamp;
      draw();
      frame = window.requestAnimationFrame(tick);
    };

    const start = (): void => {
      if (disposed || frame !== 0 || !visible) return;
      lastTimestamp = 0;
      frame = window.requestAnimationFrame(tick);
    };

    const stop = (): void => {
      if (frame !== 0) {
        window.cancelAnimationFrame(frame);
        frame = 0;
      }
    };

    const setVisible = (next: boolean): void => {
      visible = next;
      if (next) start();
      else stop();
    };

    applySize();
    start();

    const img = new Image();
    img.onload = () => {
      if (disposed) return;
      image = img;
      draw();
    };
    img.onerror = () => {
      /* keep the empty stage; the card still renders its metadata */
    };
    img.src = convertFileSrc(pet.spritesheet);

    const intersection = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) setVisible(entry.isIntersecting);
      },
      { threshold: 0.01 },
    );
    intersection.observe(stage);

    const onVisibility = (): void => setVisible(document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", onVisibility);

    const resizeObserver = new ResizeObserver(() => {
      applySize();
      draw();
    });
    resizeObserver.observe(stage);

    return () => {
      disposed = true;
      stop();
      intersection.disconnect();
      resizeObserver.disconnect();
      document.removeEventListener("visibilitychange", onVisibility);
      image = null;
    };
  }, [pet, animation, scale]);

  const drawable = Boolean(animation && animation.sprites.length > 0 && pet.spritesheet);

  return (
    <div class={className ? `pet-stage ${className}` : "pet-stage"} ref={stageRef}>
      {drawable ? (
        <canvas ref={canvasRef} aria-label={`${pet.displayName} ${t("pet.preview")}`} role="img" />
      ) : (
        <span class="small muted">{t("pet.noAnimation")}</span>
      )}
    </div>
  );
}
