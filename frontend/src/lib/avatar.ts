// A person's avatar (src/components/person-avatar.html): temper's data-tm-seed from the name, and the initials.

/** The data-tm-seed ('0'–'11') for a name: FNV-1a over its code points, so a name keeps its colour on every page. */
export function avatarSeed(name: string | null | undefined): string {
  const key = (name ?? '').trim().toLowerCase();
  return String([...key].reduce((h, ch) => Math.imul(h ^ (ch.codePointAt(0) ?? 0), 16777619) >>> 0, 2166136261) % 12);
}

/** Up to two initials: the first letters of the first and the last word ('Ada Lovelace' → 'AL'), '?' for none. */
export function initials(name: string | null | undefined): string {
  const words = (name ?? '').trim().split(/\s+/u).map((w) => w.replace(/^[^\p{L}\p{N}]+/u, '')).filter((w) => w.length > 0);
  return words.length ? ([...words[0]][0] + (words.length > 1 ? [...words[words.length - 1]][0] : '')).toUpperCase() : '?';
}
