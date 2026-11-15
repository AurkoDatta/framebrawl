import { defineConfig, devices } from "@playwright/test";

export default defineConfig({
  testMatch: "*.spec.ts",
  timeout: 120_000,
  retries: 0,
  reporter: process.env.CI ? [["list"], ["github"]] : "list",
  use: { baseURL: "http://127.0.0.1:4173" },
  webServer: [
    {
      command: "node serve.mjs",
      url: "http://127.0.0.1:4173",
      reuseExistingServer: !process.env.CI,
    },
    {
      // The game server with the built page, on ports of its own so it can
      // run next to `docker compose up`.
      command: "../../target/release/fb-server",
      env: {
        FB_HTTP_PORT: "8090",
        FB_UDP_PORT: "7690",
        FB_STATIC: "../../web/dist",
      },
      url: "http://127.0.0.1:8090/api/health",
      reuseExistingServer: !process.env.CI,
    },
  ],
  projects: [
    { name: "chromium", use: { ...devices["Desktop Chrome"] } },
    { name: "firefox", use: { ...devices["Desktop Firefox"] } },
    { name: "webkit", use: { ...devices["Desktop Safari"] } },
  ],
});
