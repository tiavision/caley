// `mailBody` helpers (src/sinter.helpers.ts): a mail body for <mail-body>.
import { progressive as numbered } from '@tiavision/sinter-mail';
import type { Body } from '@tiavision/sinter-mail';

const copies = new WeakMap<Body, Body>();

/**
 * `tree` (email.parse) numbered for progressive rendering: <mail-body> renders its first 200 nodes at once and the
 * rest in steps after a paint, so a long newsletter shows sooner. email.parse's tree is frozen and shared, and
 * sinter-mail's progressive() numbers in place, so this numbers a copy, one per tree: the same tree gives the same
 * object, which <mail-body> needs to carry its steps across renders.
 */
export function progressive(tree: Body): Body {
  let copy = copies.get(tree);
  if (!copy) {
    copy = numbered(structuredClone(tree));
    copies.set(tree, copy);
  }
  return copy;
}
