// patina's helpers, under one namespace. An app re-exports the package as a namespace
// (`export * as patina from '@tiaforge-ui/patina'` in its src/sinter.helpers.ts) and calls `patina.seed(name)` and
// `patina.initials(name)` in markup, for its own markup with data-seed. Pure and cheap: they render in the early
// script and fold at build time. <pt-avatar> writes the same two out in its lets (a library's components cannot
// call the library's helpers); test/helpers.test.ts keeps them in step.

/** The data-seed ('0'–'11') for a name: FNV-1a over its code points, so a name keeps its colour everywhere. */
export function seed(name: string | null | undefined): string {
  const key = (name ?? '').trim().toLowerCase();
  return String([...key].reduce((h, ch) => Math.imul(h ^ (ch.codePointAt(0) ?? 0), 16777619) >>> 0, 2166136261) % 12);
}

/** Up to two initials: the first letters of the first and the last word ('Ada Lovelace' → 'AL'), '?' for none. */
export function initials(name: string | null | undefined): string {
  const words = (name ?? '').trim().split(/\s+/u).map((w) => w.replace(/^[^\p{L}\p{N}]+/u, '')).filter((w) => w.length > 0);
  return words.length ? ([...words[0]][0] + (words.length > 1 ? [...words[words.length - 1]][0] : '')).toUpperCase() : '?';
}
