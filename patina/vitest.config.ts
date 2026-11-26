import { defineConfig } from 'vitest/config';

// The package's tests run against the sinter and temper installed beside it (devDependencies). The browser tests
// (contrast, rendering) use the Playwright the machine has (require('playwright')) and skip without it.
export default defineConfig({
  test: {
    include: ['test/**/*.test.ts'],
    environment: 'node',
    testTimeout: 120_000,
    hookTimeout: 180_000,
    fileParallelism: false,
    env: { SINTER_STRICT: '' },
  },
});
