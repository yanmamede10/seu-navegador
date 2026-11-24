import { defineConfig } from "vite";
import { resolve } from "node:path";

export default defineConfig({
  base: "./",
  build: {
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        shell: resolve(__dirname, "shell.html"),
        overlay: resolve(__dirname, "overlay.html"),
        newtab: resolve(__dirname, "newtab.html"),
        settings: resolve(__dirname, "settings.html"),
      },
    },
  },
});
