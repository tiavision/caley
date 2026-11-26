# Contributing

- `vocab.json` is the vocabulary. A new attribute or value goes there first, then into `sinter/patina.css`;
  `test/vocab.test.ts` fails while the two disagree, while a selector is outside `:where()` or a `pt.*` layer, while
  a colour does not come from `--tm-hue`, or while a name is close enough to one of temper's that `temper check`
  would report it as a typo.
- Colours: `test/contrast.test.ts` paints every text/surface pair in Chromium at every 15° of hue, chroma 0.04–0.22,
  light and dark, and fails under 4.5:1. Keep surfaces in their lightness band rather than lowering the bar.
- `<pt-avatar>` writes `seed` and `initials` out in its lets (a library's components cannot call the library's
  helpers); `test/avatar.test.ts` keeps them equal to `sinter.helpers.ts`.
- `npm test` runs everything, `sinter check --strict` over `fixtures/` included; `npm run typecheck`.
- Screenshots of the caley demo: `node scripts/shots.mjs <label>` with a caley build served by its DEMO_MODE backend.
