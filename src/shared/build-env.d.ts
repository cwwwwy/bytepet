/**
 * Ambient shims for the build tooling that `vite.config.ts` uses.
 *
 * `@types/node` is not a dependency of this project (and package.json is owned
 * elsewhere), yet `vite.config.ts` imports `node:path` and reads
 * `import.meta.dirname`. These declarations keep `tsc --noEmit` green without
 * touching the config itself. They describe only what the config actually uses.
 */

declare module "node:path" {
  export function resolve(...paths: string[]): string;
  export function join(...paths: string[]): string;
  export function dirname(path: string): string;
  export function basename(path: string, ext?: string): string;
}

interface ImportMeta {
  /** Node >= 20.11 / Vite 8 absolute directory of the current module. */
  readonly dirname: string;
}
