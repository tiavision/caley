# patina on caley: the demo

patina (`@tiaforge-ui/patina`, working name) is an opt-in "expressive surface" layer for temper apps. temper makes
plain semantic HTML look finished; patina gives it a voice. This demo applies it to three of caley's most visible
screens: the sign-in page, the contacts overview and a contact's detail dialog. Nothing else in caley changes: patina
is attribute-driven, so a page that uses none of its attributes renders exactly as before.

The package lives in `patina/` with its own `package.json`, tests, checker, README and llms.txt, laid out like
sinter-mail (a sinter component library: `"sinter": "./sinter"`, `sinter({ libraries: ['@tiaforge-ui/patina'] })`),
so it can be lifted into its own repository unchanged. caley consumes it as `"@tiaforge-ui/patina": "file:../patina"`.

## Screenshots

Taken with `node scripts/shots.mjs before|after` (Playwright, Chromium) against caley's build served by the
DEMO_MODE backend: 1280 × 800 and 390 × 844, light and dark, reduced motion.

### Sign-in page

| | Before | After |
|---|---|---|
| Desktop, light | ![](screenshots/login-before-desktop-light.png) | ![](screenshots/login-after-desktop-light.png) |
| Desktop, dark | ![](screenshots/login-before-desktop-dark.png) | ![](screenshots/login-after-desktop-dark.png) |
| Phone, light | ![](screenshots/login-before-phone-light.png) | ![](screenshots/login-after-phone-light.png) |
| Phone, dark | ![](screenshots/login-before-phone-dark.png) | ![](screenshots/login-after-phone-dark.png) |

A mesh background with grain, the name in Fraunces (the `editorial` pairing) with gradient ink, the sign-in card in
glass with a floating shadow and a haloed button, and what Caley does as a bento grid of glass tiles, each icon on a
seeded gradient.

### Contacts overview

| | Before | After |
|---|---|---|
| Desktop, light | ![](screenshots/contacts-before-desktop-light.png) | ![](screenshots/contacts-after-desktop-light.png) |
| Desktop, dark | ![](screenshots/contacts-before-desktop-dark.png) | ![](screenshots/contacts-after-desktop-dark.png) |
| Phone, light | ![](screenshots/contacts-before-phone-light.png) | ![](screenshots/contacts-after-phone-light.png) |
| Phone, dark | ![](screenshots/contacts-before-phone-dark.png) | ![](screenshots/contacts-after-phone-dark.png) |

A mesh masthead with the page's title (it had no `h1` before) and the counts as stat tiles (`data-pt-figure`), a glass
toolbar that stays at the top while the list scrolls under it, the letters in display type, raised cards that lift
on hover, and `<pt-avatar>`: two initials on the name's seeded gradient, so each person keeps a colour.

### Contact detail

| | Before | After |
|---|---|---|
| Desktop, light | ![](screenshots/contact-detail-before-desktop-light.png) | ![](screenshots/contact-detail-after-desktop-light.png) |
| Desktop, dark | ![](screenshots/contact-detail-before-desktop-dark.png) | ![](screenshots/contact-detail-after-desktop-dark.png) |
| Phone, light | ![](screenshots/contact-detail-before-phone-light.png) | ![](screenshots/contact-detail-after-phone-light.png) |
| Phone, dark | ![](screenshots/contact-detail-before-phone-dark.png) | ![](screenshots/contact-detail-after-phone-dark.png) |

The name in display type, a gradient band with the large haloed avatar and the organization, a floating dialog.

### A rebrand is one number

The same build with only `--tm-hue: 160` added (caley's is 280). The mesh, the ink, the seeds, the shadows' tint and
temper's own accent all follow.

| Light | Dark |
|---|---|
| ![](screenshots/login-rebrand-hue160-desktop-light.png) | ![](screenshots/login-rebrand-hue160-desktop-dark.png) |
| ![](screenshots/contacts-rebrand-hue160-desktop-light.png) | ![](screenshots/contacts-rebrand-hue160-desktop-dark.png) |

## The vocabulary I settled on

Twelve attributes, all in patina's own `data-pt-*` namespace, all in [`vocab.json`](../vocab.json) (temper's format), all checked.

| Attribute | Values | |
|---|---|---|
| `data-pt-surface` | glass, gradient, mesh | The background as a material. Panels (not body/main) get temper's large radius. |
| `data-pt-grain` | (boolean) | Film grain over any surface. |
| `data-pt-elevation` | raised, floating | Brand-tinted shadows; a raised card with a `data-stretch` control lifts on hover. |
| `data-pt-halo` | (boolean) | A soft accent glow. |
| `data-pt-display` | hero, xl, lg | Display type: fluid sizes, tight tracking, balanced lines, the pairing's face. |
| `data-pt-ink` | gradient | Gradient text for display sizes. |
| `data-pt-figure` | (boolean) | A big tabular number (stat tiles), labelled with temper's `data-caption`. |
| `data-pt-hero` | (boolean) | A centered hero column whose padding grows with the screen. |
| `data-pt-bento` | (boolean) | With temper's `data-layout="grid"`: row height, dense flow, spans. |
| `data-pt-span` | wide, tall, large, full | A bento tile's size (from a 32rem grid up). |
| `data-pt-pairing` | editorial, grotesk | Fraunces or Bricolage Grotesque for display, self-hosted through Fontsource. |
| `data-pt-seed` | 0 … 11 | Twelve gradient fills (and inks) spread ±82.5° around the brand hue. |

One component, `<pt-avatar>` (name, photo, size, halo), and two helpers, `patina.seed(name)` and
`patina.initials(name)`. Five input tokens (`--pt-font-display`, `--pt-display-weight`, `--pt-display-tracking`,
`--pt-blur`, `--pt-row`), nine derived ones.

How it keeps the ecosystem's grain:

- **Checked vocabulary.** `patina check` (CLI, `@tiaforge-ui/patina/check`, and a Vite plugin) runs temper's own
  checker with patina's `vocab.json` as `ctx.vocab`, keeps the findings about patina's attributes and renumbers them
  PT1xx (bad value, boolean with a value, typo, missing companion such as `data-pt-bento` without
  `data-layout="grid"`, wrong element, `data-pt-span` outside a bento, `patina.css` not imported), plus PT2xx for
  `--pt-*` tokens. temper's own rules stay with `temper check`, so nothing is reported twice. caley's build runs
  both: `temper check: 0 error(s), 2 warning(s)` (the same two as before) and `patina check: 0 error(s), 0 warning(s)`.
- **Its own layers.** `@layer tm, pt;` puts patina after temper whatever the import order; every rule is in
  `pt.base`/`pt.surface`/`pt.type`/`pt.layout`, every selector in `:where()`. caley's unlayered CSS still wins.
- **Two numbers.** Every colour is `oklch()` of `--tm-hue`/`--tm-chroma` (a test fails on any hex/rgb/hsl or an
  oklch() that does not read the hue). Surfaces cap their chroma at 0.2 so a loud brand does not shout.
- **AA in light and dark.** `test/contrast.test.ts` resolves the tokens in Chromium, paints each colour on a canvas
  (so out-of-gamut colours count as drawn), composites glass over each backdrop it is meant for, and checks 4.5:1 for
  temper's text, muted text and accent, patina's two gradient inks and the twelve seed inks, on temper's background
  and surface, the three tints and glass over each: every 15° of hue × chroma 0.04, 0.12, 0.19, 0.22 × light and dark.
  The worst pair is ≥ 4.5. temper-inspect on the built pages: no problems on `/` and `/contacts/`; over all pages
  0 errors (its 15 warnings are the "opened directly" kind CLAUDE.md lists as known gaps).
- **Preferences.** `prefers-contrast: more` and forced colours: plain temper surfaces, solid text, no grain;
  `prefers-reduced-transparency`: opaque glass; `prefers-reduced-motion`: no lift. Tested by emulating the media
  features in Chromium.
- **No app CSS, no runtime JS, first frame.** caley's `login.scss` lost its brand and heading rules (31 → 13 lines,
  what is left is the page grid it had); `contacts.scss` lost one rule (the toolbar is now positioned by temper's
  `data-sticky`). No CSS was added. `<pt-avatar>` has no script: it renders in sinter's early script with the rest,
  and the Fraunces face is preloaded by sinter (`fonts.preload`) on the two pages that import it.

Cost: `patina.css` is 7.9 kB minified, 2.2 kB gzip, in the shared stylesheet. The Fraunces latin face is 36.6 kB,
loaded only by the sign-in and contacts pages.

## What I cut, and why

- **`data-surface="grain"`.** Grain is a texture over a material, not a material, so it became the boolean
  `data-pt-grain` that combines with any surface (and the page background).
- **`data-layout="bento"`.** temper owns `data-layout` and its fixed values; temper check rejects `bento` (TM101).
  patina refines temper's grid instead: `data-layout="grid" data-pt-bento`, which also keeps temper's `data-gap` and
  `--tm-min` working.
- **Unprefixed names** (`data-surface`, `data-glow`, …), the first version. They shared temper's namespace: a name one
  edit from temper's (`data-glow` ↔ `data-grow`) was reported by temper check as a typo, and nothing told a reader
  which library an attribute belongs to. Every patina attribute is now `data-pt-*` (as sinter's are `data-sn-*`), so
  none can collide with temper's, and the checker reports every unknown `data-pt-*` name as an error instead of
  guessing typos.
- **A stat-tile component.** Markup is enough: a glass tile with `dt data-caption` and `dd data-pt-figure`. A component
  would only hide a `dl`.
- **Scroll-reveal animations** (`animation-timeline: view()`). They start content hidden, which breaks "everything in
  the first frame", and add motion that reduced-motion users must opt out of.
- **Elevation levels beyond two, and per-element glow colours.** `raised` and `floating` covered every case on the
  three screens; a glow per seed was noise next to the seeded fills.
- **A third pairing preset.** Two (a serif, a grotesque) show the mechanism; caley uses `editorial`. More are a
  line each in the vocabulary and the CSS.
- **High-chroma brands.** Above `--tm-chroma` 0.22, temper's own accent nears 4.5:1 on any light tint; rather than
  lower the bar, the contrast guarantee is stated for 0.04–0.22 (temper's default is 0.19).

## What temper and sinter would need

Found while building this; neither repository was changed.

### temper

1. **Extension vocabularies in `temper check`.** temper check knows only its own attributes. It skips `data-sn-*`
   (sinter's) when it looks for typos, but not another library's prefix, so `data-pt-*` names are safe today only
   because they are far from temper's; and a package cannot add a value to one of temper's attributes
   (`data-layout="bento"`). An option such as
   `temper({ vocabularies: ['@tiaforge-ui/patina/vocab.json'] })` would let one checker run (and prune, and feed the
   editor's `html-custom-data.json`) over both, and make patina's separate checker unnecessary.
2. **`checkStyles` hard-codes `--tm-`.** A `prefix` in the vocabulary would let it check `--pt-*` tokens too; patina
   reimplements that part.
3. **`lib/check.js` takes `ctx.vocab` "for tests".** patina depends on it; making it public (and stable) API is what
   allows a layer on top at all.
4. **`prune: true` and other layers.** temper's pruning scans for its own attributes only; with an extension
   vocabulary it could prune patina's unused values the same way.
5. **Radius tokens.** patina rounds panels with `--tm-radius-lg`, which exists but is not listed among the documented
   tokens in llms.txt; list it (and `--tm-radius-sm`).

### sinter

1. **A library's components cannot call the library's own helpers.** `seed(name)` in `pt-avatar.html` is SN201: the
   helpers are only reachable through the namespace the app chooses (`patina.seed`), which the library cannot know.
   `<pt-avatar>` therefore writes both helpers out in its lets, and a test keeps the two copies equal. Libraries need
   their own helpers in scope in their components (by bare name, or a fixed namespace of the library's choosing).
2. **`render()` in `@tiaforge-ui/sinter/testing` has no `libraries` option.** caley's `src/test-components.ts`
   passes sinter-mail's and now patina's components as if they were the app's.
3. **Fonts per page.** `fonts.preload` names faces for the whole build; the display face should be preloaded only
   where a pairing is used. It works because only the two pages import Fraunces, but an attribute-level hook
   (`data-pt-pairing` → its faces) would be clearer.
4. **Global CSS from a library.** A library can ship scoped component styles (`sinter-lib` layer) but its global
   attribute CSS is a separate import the app must remember (`patina.css`, like sinter-mail's `mail.css`). A
   `"sinter": { "styles": "./patina.css" }` entry that the plugin injects for every configured library would remove
   the step (patina's checker reports it as PT110 meanwhile).

## Verification

- `patina/`: `npm test` (26 tests: vocabulary ↔ CSS, names clear of temper, the checker and its CLI,
  `sinter check --strict` over the fixtures, the helpers and `<pt-avatar>`, contrast at every hue, the
  preferences), `npm run typecheck`. CI runs both in a new `patina` job.
- `frontend/`: `npm run build` (sinter check strict, temper check, patina check), `npm test`: all pass except three
  tests in `settings/index.test.ts` and `src/components/import-dialog.test.ts` that fail the same way without these
  changes under this machine's default Node 22 (jsdom's `FormData`) and pass under Node 24 (CI uses Node 26). The contacts tests were updated for
  the new markup and one was added for the masthead's counts.
- temper-inspect (`node scripts/inspect.mjs`) over every page of the build: 0 errors.
- The Docker image copies `patina/` before `npm ci`, since the frontend links it.
