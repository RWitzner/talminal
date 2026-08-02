import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig(({ mode }) => {
  const perfEnabled = loadEnv(mode, process.cwd(), "").VITE_TALMINAL_PERF === "1";
  return {
    plugins: [react()],
    clearScreen: false,
    // A free identifier is replaced with a literal at every callsite. Unlike
    // an imported const, this lets the minifier erase the entire disabled
    // branch, its arguments, clocks, event strings, and perfTrace wire fields.
    define: {
      PERF_ENABLED: JSON.stringify(perfEnabled),
    },
    server: {
      port: 1420,
      strictPort: true,
      watch: {
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
