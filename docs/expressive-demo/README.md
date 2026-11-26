# temper's expressive module on caley

temper makes plain semantic HTML look finished; its opt-in `expressive.css` module gives it a voice: surfaces,
brand-tinted depth, display type, hero and bento layouts, seeded colour. This demo applies it to three of caley's
most visible screens: the sign-in page, the contacts overview and a contact's detail dialog. Pages that use none of
its attributes render as before.

**Where it lives.** The module is on temper's branch
[`claude/data-tm-prefix`](https://github.com/tiaforge/temper/tree/claude/data-tm-prefix), together with the rename
of every temper attribute to `data-tm-*`. caley installs temper from that branch (a git dependency; temper builds
its CSS in `prepare`) until it is released as 0.5, then goes back to a version range.

**How it got there.** It started as a separate sinter library in this repository (`patina`, attributes
`data-pt-*`). Nearly everything it needed from temper (extension vocabularies for the checker, a second token
prefix, pruning) existed only because it was separate, so it moved into temper as a module, and temper's attributes
moved into their own namespace at the same time (`data-variant` → `data-tm-variant`, as sinter's are `data-sn-*`).
The one sinter-specific piece, the avatar component, is now caley's `<person-avatar>`.

## Screenshots

`node docs/expressive-demo/shots.mjs before|after` (Playwright, Chromium) against caley's build served by the
DEMO_MODE backend: 1280 × 800 and 390 × 844, light and dark, reduced motion.

### Sign-in page

| | Before | After |
|---|---|---|
| Desktop, light | ![](screenshots/login-before-desktop-light.png) | ![](screenshots/login-after-desktop-light.png) |
| Desktop, dark | ![](screenshots/login-before-desktop-dark.png) | ![](screenshots/login-after-desktop-dark.png) |
| Phone, light | ![](screenshots/login-before-phone-light.png) | ![](screenshots/login-after-phone-light.png) |
| Phone, dark | ![](screenshots/login-before-phone-dark.png) | ![](screenshots/login-after-phone-dark.png) |

Caley's lockup as in the rail, the tagline as the page's one display moment (Fraunces, the `editorial` pairing, at
weight 500), the sign-in button on its own, and what Caley does as a plain row of four (a list on a phone), under a
wash of the brand colour at the top. A first version (full-page mesh with grain, a gradient-ink "Caley", a glass
card around the button, a bento of glass feature tiles) read as a generic landing page and was reworked in a design
review.

### Contacts overview

| | Before | After |
|---|---|---|
| Desktop, light | ![](screenshots/contacts-before-desktop-light.png) | ![](screenshots/contacts-after-desktop-light.png) |
| Desktop, dark | ![](screenshots/contacts-before-desktop-dark.png) | ![](screenshots/contacts-after-desktop-dark.png) |
| Phone, light | ![](screenshots/contacts-before-phone-light.png) | ![](screenshots/contacts-after-phone-light.png) |
| Phone, dark | ![](screenshots/contacts-before-phone-dark.png) | ![](screenshots/contacts-after-phone-dark.png) |

The page's one expressive moment is its top: a wash of the brand colour behind the title (it had no `h1` before), in
the display face, with the counts in one quiet line. Everything people work with (the search field, the letters,
the cards) stays plain temper; the cards are raised and lift on hover, and `<person-avatar>` puts two initials on
the name's seeded gradient, so each person keeps a colour. The page's padding is now 16px (was 8px), which aligns the
title with the list and gives the cards more room.

### Contact detail

| | Before | After |
|---|---|---|
| Desktop, light | ![](screenshots/contact-detail-before-desktop-light.png) | ![](screenshots/contact-detail-after-desktop-light.png) |
| Desktop, dark | ![](screenshots/contact-detail-before-desktop-dark.png) | ![](screenshots/contact-detail-after-desktop-dark.png) |
| Phone, light | ![](screenshots/contact-detail-before-phone-light.png) | ![](screenshots/contact-detail-after-phone-light.png) |
| Phone, dark | ![](screenshots/contact-detail-before-phone-dark.png) | ![](screenshots/contact-detail-after-phone-dark.png) |

The contact's identity in one header: the seeded avatar, the name as the dialog's one display moment and the
organization under it, the actions at the end (on a phone, on their own row so the name keeps the width). A first
version with a gradient band and a glowing avatar read as a social-profile template and was removed in the review.

### A rebrand is one number

The same build with only `--tm-hue: 160` added (caley's is 280).

| Light | Dark |
|---|---|
| ![](screenshots/login-rebrand-hue160-desktop-light.png) | ![](screenshots/login-rebrand-hue160-desktop-dark.png) |
| ![](screenshots/contacts-rebrand-hue160-desktop-light.png) | ![](screenshots/contacts-rebrand-hue160-desktop-dark.png) |

## The vocabulary

Twelve attributes in temper's `vocab.json` (module `expressive`), checked by `temper check` like every other.

| Attribute | Values | |
|---|---|---|
| `data-tm-surface` | glass, gradient, mesh, wash | The background as a material. Panels (not body/main) get temper's large radius; on `<body>` it covers the canvas. `wash` is the mesh at the top of a working view only. |
| `data-tm-grain` | (boolean) | Film grain over any surface (an inline SVG: the CSP needs `img-src data:`, which caley's has). |
| `data-tm-elevation` | raised, floating | Brand-tinted shadows; a raised card with a `data-tm-stretch` control lifts on hover. |
| `data-tm-halo` | (boolean) | A soft accent glow. |
| `data-tm-display` | hero, xl, lg | Display type: fluid sizes, tight tracking, balanced lines, the pairing's face. |
| `data-tm-ink` | gradient | Gradient text for display sizes. |
| `data-tm-figure` | (boolean) | A big tabular number (stat tiles), labelled with `data-tm-caption`. |
| `data-tm-hero` | (boolean) | A centered hero column whose padding grows with the screen (`data-tm-pad` fixes it). |
| `data-tm-bento` | (boolean) | With `data-tm-layout="grid"`: row height (`--tm-bento-row`), dense flow, spans. |
| `data-tm-span` | wide, tall, large, full | A bento tile's size (from a 32rem grid up). |
| `data-tm-pairing` | editorial, grotesk | Fraunces or Bricolage Grotesque for display, self-hosted by the app through Fontsource. |
| `data-tm-seed` | 0 … 11 | Twelve gradient fills (and inks) spread ±50° around the brand hue. |

Tokens to set: `--tm-font-display`, `--tm-display-weight`, `--tm-display-tracking`, `--tm-glass-blur`,
`--tm-bento-row`. Read only: `--tm-tint-1…3`, `--tm-glass`, `--tm-ink-1…2`, `--tm-glow`, `--tm-shadow-raised`,
`--tm-shadow-floating`.

What it guarantees, tested in temper (`test/expressive.test.js`): every colour is `oklch()` of `--tm-hue`/`--tm-chroma`;
temper's text, muted text and accent, the gradient ink and the twelve seed inks keep 4.5:1 on every surface and on
glass over each, in light and dark mode, at every 15° of hue for chroma 0.04–0.22 (temper's default is 0.19);
`prefers-contrast: more` and forced colours give plain surfaces and solid text, `prefers-reduced-transparency` opaque
glass, `prefers-reduced-motion` no lift. No script; 1.9 KB brotli. The Fraunces face (36.6 kB) loads only on the two
pages that import it, preloaded by sinter.

## The design review of the contacts page

The first version had a boxed mesh masthead with gradient-ink title and three glass stat tiles. It looked pasted in,
especially in dark mode. A review against design fundamentals (one place for boldness; big-number tiles only when the
numbers are the point; no purple-to-blue gradient defaults; cards by role) and a second reviewer looking only at the
screenshots led, over five rounds, to:

- the colour moved from a box to a `wash` at the top of the view (new in temper), with an eased falloff, 60% strength
  in dark and no seam at the scrollbar;
- the mesh's hues narrowed to the brand hue ±20° (it read as blue and magenta) and the light tints made paler (no cyan);
- one display moment: the title at `lg`; the counts as one small line; toolbar and letters back to plain temper;
- the avatar seeds narrowed to ±50° and calmer in dark (no red or maroon);
- the title aligned with the list.

The sign-in page, the contact dialog and temper's own reference page for the module (`kitchen-sink/expressive.html`)
went through the same review afterwards; temper's reference page now teaches the restraint (two worked examples, then
each part with when to use it) instead of the template look.

Out of scope and unchanged: the floating add button can cover the last card on a phone, as before.

## The rest of caley is unchanged

Calendar, mail (list and an open message), search and settings were compared pixel by pixel with `main`, light and
dark, desktop and phone: identical, apart from settings on desktop, where temper's whole-pixel centring (already on
temper's `main`) moves the column by half a pixel. That comparison also found that caley's `settings.scss` set the
column's padding without temper's centring knowing, which would have put it 8px off centre with any newer temper; it
now sets temper's side-padding variable instead.

## What changed in caley

- Every temper attribute is `data-tm-*` (markup, tests, SCSS selectors, `dataset` in scripts, docs). `temper check`
  now errors on a temper name without the prefix (and `--fix` adds it), so none is left.
- The three screens use the module; caley's `login.scss` lost its brand and heading rules (31 → 13 lines) and
  `contacts.scss` one rule. No CSS was added.
- `<person-avatar>` (`src/components/person-avatar.html`) with `avatarSeed` and `initials` (`src/lib/avatar.ts`).
- New messages (en, de) for the login features and the contacts counts; the contacts page has an `h1`.
- Verification: `npm run build` (sinter strict, temper check: 0 errors, the 2 warnings it had before), `npm test`
  (228 pass; 3 settings/import tests fail on this machine's Node 22 as they do without these changes, and pass on
  Node 24), temper-inspect over every page: 0 errors (15 "opened directly" warnings, the known kind).

## What was cut, and why

- **Grain as a surface value.** It is a texture over a material, so it is the boolean `data-tm-grain`.
- **`data-tm-layout="bento"`.** `data-tm-bento` refines temper's grid instead, so `data-tm-gap` and `--tm-min` keep
  working and `data-tm-layout` stays one list.
- **A stat-tile component.** A glass tile with `dt data-tm-caption` and `dd data-tm-figure` is enough.
- **Scroll-reveal animations.** They start content hidden (not in the first frame) and add motion.
- **More elevation levels, per-seed glows, a third pairing.** Not needed on the three screens; each is a line in
  the vocabulary and the CSS if wanted later.
- **High-chroma brands.** Above `--tm-chroma` 0.22 temper's own accent nears 4.5:1 on light tints; the guarantee is
  stated for 0.04–0.22 rather than lowering the bar.

## Still open

- **temper-inspect.** caley's devDependency is still the published 0.4 temper-inspect; its overflow check looks for
  `[data-sr-only]`, renamed `[data-tm-sr-only]` on the branch. It ships with temper's next release.
- **sinter: fonts per page.** `fonts.preload` names faces for the whole build; it works here because only the two
  pages import Fraunces. A way to tie a face to `data-tm-pairing` would be clearer.
- **Release.** Once temper 0.5 is out, caley's `package.json` goes back to `"@tiaforge-ui/temper": "^0.5.0"`.
