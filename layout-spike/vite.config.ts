import tailwindcss from "@tailwindcss/vite";
import vue from "@vitejs/plugin-vue";
import { defineConfig } from "vite";

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  // Vite resolves a leading-slash alias from the project root, so this needs no `node:url`.
  resolve: {
    alias: {
      "@": "/src",
    },
  },
  clearScreen: false,
});
