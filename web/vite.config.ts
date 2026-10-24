import { defineConfig } from "vite";
export default defineConfig({
  server: { proxy: { "/api": "http://127.0.0.1:9080" } },
  build: { target: "es2022" },
});
