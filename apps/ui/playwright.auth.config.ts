import { defineConfig } from '@playwright/test';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
export default defineConfig({
  testDir: './tests', outputDir: './test-results-auth', testMatch: ['login-enrollment.spec.ts', 'group-membership.spec.ts'], fullyParallel: false, workers: 1, timeout: 45_000,
  use: { baseURL: 'http://127.0.0.1:3108', viewport: { width: 1440, height: 1000 }, trace: 'retain-on-failure' },
  webServer: { command: `node ../../scripts/dev-ui.mjs --port 3108 --data-dir "${mkdtempSync(path.join(tmpdir(), 'croaker-auth-test-'))}"`, env: { CROAKER_DEMO_AUTOLOGIN: '0', CROAKER_LOGIN_MODE: 'demo' }, url: 'http://127.0.0.1:3108/healthz', reuseExistingServer: false, timeout: 180_000, gracefulShutdown: { signal: 'SIGINT', timeout: 10_000 } },
});
