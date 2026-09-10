import { defineConfig } from "vite";
import preact from "@preact/preset-vite";
import { resolve } from "node:path";

const root = import.meta.dirname;

// Two HTML entries:
//   pet.html  -> transparent always-on-top overlay (plain TS + canvas, no framework)
//   chat.html -> chat + settings window (Preact)
export default defineConfig({
  plugins: [preact()],
  clearScreen: false,
  server: {
    port: 5273,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**", "**/target/**", "**/.cache/**"] },
  },
  build: {
    target: "es2022",
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        pet: resolve(root, "pet.html"),
        chat: resolve(root, "chat.html"),
      },
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
