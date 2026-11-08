// DOM lookups for handlers that move the focus or scroll (`elementIn($el, '.pane')?.scrollTo(0, 0)`):
// they read the document, never change it, and type the result as HTMLElement so handlers need no casts.

/** The first element under `root` matching `selector`, or null. */
export function elementIn(root: ParentNode, selector: string): HTMLElement | null {
  return root.querySelector<HTMLElement>(selector);
}

/** The closest ancestor of `el` (or `el` itself) matching `selector`, or null, e.g. the menu a button is in. */
export function ancestor(el: Element, selector: string): HTMLElement | null {
  return el.closest<HTMLElement>(selector);
}
