// Keyboard shortcuts of visible controls: `data-shortcut="<key>"` on a button names the key that clicks it
// (src/islands/common.ts listens). Pure parts here, unit-tested.

/**
 * Whether a keydown matches a `data-shortcut` value: modifiers first, the key last, joined by `-` as in sinter's
 * key filters, e.g. `c`, `shift-i`, `#`, `escape`. Ctrl, Alt and Meta must be exactly the listed ones. Letters and
 * named keys check Shift too (`n` is not Shift+N), other characters do not (`#` however it is typed).
 */
export function shortcutMatches(spec: string, e: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'altKey' | 'metaKey' | 'shiftKey'>): boolean {
  const parts = spec.length > 1 ? spec.split('-') : [spec];
  const key = parts.pop() ?? '';
  const mods = new Set(parts);
  if (e.ctrlKey !== mods.has('ctrl') || e.altKey !== mods.has('alt') || e.metaKey !== mods.has('meta')) return false;
  const letterOrName = /^[a-z]$/i.test(key) || key.length > 1;
  if (!letterOrName) return e.key === key;
  return e.key.toLowerCase() === key.toLowerCase() && e.shiftKey === mods.has('shift');
}

/** Whether a keydown happened in a place that takes typing (fields, contenteditable): shortcuts leave those alone. */
export function typingTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  return !!target.closest('input, textarea, select, [contenteditable=""], [contenteditable="true"]');
}
