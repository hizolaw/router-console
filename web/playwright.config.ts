import { defineConfig } from "@playwright/test";
export default defineConfig({
  testDir: "./tests",
  use: {
    baseURL: "http://127.0.0.1:19080",
    launchOptions: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE
      ? { executablePath: process.env.PLAYWRIGHT_CHROMIUM_EXECUTABLE }
      : {},
  },
  webServer: {
    command: "../target/release/router-console --config tests/console.toml",
    url: "http://127.0.0.1:19080/",
    env: { ROUTER_CONSOLE_TOKEN: "test-only-public-fixture-token-12345" },
    reuseExistingServer: false,
  },
});
