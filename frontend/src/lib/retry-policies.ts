// The retry policies (src/sinter.config.ts), also configured by the page tests whose markup names one.
// Sinter sends each request once. A load that hit a network error (a phone between networks) is tried
// again a few times; so are the writes whose elements opt in with data-sn-retry="flags": the mail
// flag PATCHes, which set a flag to a value and so may arrive twice. The backend ignores the
// Idempotency-Key header (no write that needs it opts in).
import { retry } from '@tiaforge-ui/sinter/retry';

export const retryPolicies = retry({
  policies: {
    network: { backoff: ['1s', '10s'], retries: 3 },
    flags: { backoff: ['1s', '30s'], maxAge: '5m' },
  },
  get: 'network',
});
