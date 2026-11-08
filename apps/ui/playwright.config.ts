import { defineConfig } from '@playwright/test';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const dataDir = mkdtempSync(path.join(tmpdir(), 'croaker-ui-test-'));
export default defineConfig({
  testDir: './tests', testIgnore: ['login-enrollment.spec.ts', 'group-membership.spec.ts', 'browser-client.spec.ts'], fullyParallel: false, workers: 1, timeout: 45_000,
  use: { baseURL: 'http://127.0.0.1:3107', viewport: { width: 1440, height: 1000 }, trace: 'retain-on-failure' },
  webServer: {
    env: { CROAKER_DEMO_AUTOLOGIN: '1', CROAKER_LOGIN_MODE: 'demo' },
    command: `node ../../scripts/dev-ui.mjs --port 3107 --data-dir "${dataDir}"`,
    url: 'http://127.0.0.1:3107/healthz', reuseExistingServer: false,
    timeout: 180_000, gracefulShutdown: { signal: 'SIGINT', timeout: 10_000 },
  },
});
