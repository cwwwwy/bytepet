/**
 * Pulls in vitest's `declare module "vite" { interface UserConfig { test } }`
 * augmentation so `defineConfig({ test: ... })` in vite.config.ts type-checks.
 * Vite's own `defineConfig` has no `test` field; vitest adds it on import.
 */

import "vitest/config";
