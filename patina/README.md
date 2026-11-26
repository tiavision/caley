# @tiaforge-ui/patina

An expressive surface layer for [temper](https://github.com/tiaforge/temper) apps built with
[sinter](https://github.com/tiaforge/sinter). temper makes plain semantic HTML look *finished*; patina gives it a
*voice*: materials, brand-tinted depth, display type, hero and bento layouts, seeded colour.

```html
<body data-surface="mesh" data-grain data-pairing="editorial">
  <header data-hero>
    <h1 data-display="hero" data-ink="gradient">Caley</h1>
    <p data-muted>Calendar, contacts and mail on your own servers.</p>
    <article data-surface="glass" data-elevation="floating" data-pad="lg"><button data-halo>Enter</button></article>
  </header>
  <ul data-layout="grid" data-bento data-gap="md">
    <li data-surface="glass" data-span="wide">…</li>
  </ul>
  <pt-avatar data-sn-with="{ name: c.fn_name, src: c.photo }"></pt-avatar>
</body>
```

It keeps temper's grain:

- **A fixed vocabulary.** Twelve `data-*` attributes with fixed values, listed in [vocab.json](vocab.json) (temper's
  format) and checked by `patina check`, which is temper's own checker run with patina's vocabulary.
- **Its own cascade layers.** Every rule is in `@layer pt.*`, after temper's `tm.*` layers, every selector in
  `:where()`. The app's own CSS always wins without `!important`; the app needs no CSS of its own for anything
  patina draws.
- **Two numbers are the brand.** Every colour is OKLCH from temper's `--tm-hue` and `--tm-chroma`. Change them and
  the mesh, the glass, the gradient ink, the shadows and the twelve avatar seeds all follow.
- **Text keeps WCAG AA.** temper's text, muted text and accent, and patina's gradient ink and seed inks, keep 4.5:1
  on every patina surface, in light and dark mode, at every hue (checked every 15°) for `--tm-chroma` 0.04–0.22
  (temper's default is 0.19). The test paints the colours in Chromium and measures the pixels.
- **Preferences.** `prefers-contrast: more` and forced colours turn every surface into temper's plain one and the
  gradient ink into solid text; `prefers-reduced-transparency` makes glass opaque; `prefers-reduced-motion` stops
  the hover lift.
- **No runtime script, first frame.** It is CSS plus one sinter component that renders in sinter's early script; the
  display faces are self-hosted and preloaded by sinter.

## Install

```sh
npm install @tiaforge-ui/patina
# a display face for a pairing, if you use one:
npm install @fontsource-variable/fraunces          # data-pairing="editorial"
npm install @fontsource-variable/bricolage-grotesque  # data-pairing="grotesk"
```

Needs sinter 0.22 and temper 0.4 or later.

```js
// vite.config.js
import sinter from '@tiaforge-ui/sinter/vite';
import temper from '@tiaforge-ui/temper/vite';
import patina from '@tiaforge-ui/patina/vite';

export default {
  plugins: [
    sinter({ libraries: ['@tiaforge-ui/patina'], fonts: { preload: ['Fraunces Variable'] } }),
    temper(),
    patina({ paths: ['src', 'index.html'] }), // patina check in every build; errors fail it
  ],
};
```

```ts
// your styles entry, after temper
import '@tiaforge-ui/temper/core.css';
import '@tiaforge-ui/patina/patina.css';
import '@fontsource-variable/fraunces';

// src/sinter.helpers.ts: patina.seed(name), patina.initials(name) for your own markup
export * as patina from '@tiaforge-ui/patina';

// src/sinter.types.ts
export type { AvatarSize } from '@tiaforge-ui/patina';
```

## Vocabulary

| Attribute | Values | What it does |
|---|---|---|
| `data-surface` | glass, gradient, mesh | The background as a material. A surface other than `body`/`main` gets temper's large radius. |
| `data-grain` | (boolean) | Faint film grain over the background (an inline SVG image: the CSP needs `img-src data:`). |
| `data-elevation` | raised, floating | Shadows tinted with the brand hue. A raised card with a `data-stretch` control lifts on hover. |
| `data-halo` | (boolean) | A soft glow of the accent around the element. |
| `data-display` | hero, xl, lg | Display type: the pairing's display face, fluid size, tight tracking, balanced lines. |
| `data-ink` | gradient | Brand-gradient text (with `data-display` or `data-figure`). |
| `data-figure` | (boolean) | A big tabular number for stat tiles; pair it with temper's `data-caption`. |
| `data-hero` | (boolean) | A centered hero column with padding that grows with the screen (`data-pad` fixes it). |
| `data-bento` | (boolean) | With `data-layout="grid"`: rows of `--pt-row`, dense filling, tiles sized by `data-span`. |
| `data-span` | wide, tall, large, full | A bento tile's size (from a 32rem wide grid up; below, one cell). |
| `data-pairing` | editorial, grotesk | The display face: Fraunces or Bricolage Grotesque, with its weight and tracking. |
| `data-seed` | 0 … 11 | A gradient fill and ink from one of twelve hues around the brand hue (avatars, icon tiles). |

Tokens to set: `--pt-font-display`, `--pt-display-weight`, `--pt-display-tracking`, `--pt-blur`, `--pt-row`. Derived
(read, never set): `--pt-tint-1…3`, `--pt-glass`, `--pt-ink-1…2`, `--pt-glow`, `--pt-shadow-raised`,
`--pt-shadow-floating`.

### `<pt-avatar>`

```html
<pt-avatar data-sn-with="{ name: c.fn_name, src: c.has_photo ? photoUrl(c) : null, size: 'lg', halo: false }"></pt-avatar>
```

temper's `data-avatar` with the photo, or up to two initials on the name's seeded gradient: a person keeps their
colour on every page. `size` is `sm`, `md`, `lg` (temper's) or `xl` (4.5rem). Hidden from screen readers, since the
name is beside it.

## Checking

```sh
npx patina check src index.html   # or the Vite plugin
```

| Code | |
|---|---|
| PT101 | A value patina does not have (`data-surface="frosted"`), with the allowed values. |
| PT102 | A value on a boolean attribute. |
| PT104 | A typo of a patina attribute (`data-surfase`). |
| PT107 | A companion attribute is missing (`data-bento` without `data-layout="grid"`). |
| PT108 | An attribute on an element it does nothing on (`data-display` on a `<ul>`). |
| PT109 | A `data-span` outside a bento grid. |
| PT110 | patina attributes are used but `patina.css` is not imported. |
| PT201–203 | An unknown `--pt-*` token, a `--pt-` name of your own, or a derived token set by the app. |

temper's own rules (labels, structure) stay with `temper check`, so the two never report one thing twice. Values
bound by sinter (`data-sn-attr="{ 'data-seed': … }"`) are checked like static ones.

## Limits

- Glass keeps AA over patina's surfaces and the page background. Over busy content (text scrolling under a sticky
  glass bar) the blur and a 72% fill keep it readable, not measured.
- Above `--tm-chroma` 0.22 temper's own accent nears 4.5:1 on light tints; the contrast test stops there.
- Bento spans need a 32rem wide grid (a container query: `--tm-min` cannot be read there).

## License

MIT
