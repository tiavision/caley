// The words of alloy's <alloy-sortable> in the page's language (its `text` prop): the handle's label and
// description, and what its live region announces while a row is moved (the page's own alert says when a saved order is refused).
import { m } from '@tiaforge-ui/sinter/messages';
import type { SortableText } from '@tiavision/sinter-alloy';

export function sortText(): SortableText {
  return {
    handle: (name) => m.sort_handle({ name }),
    instructions: m.sort_instructions(),
    lifted: (name, position, total) => m.sort_lifted({ name, position, total }),
    moved: (name, position, total) => m.sort_moved({ name, position, total }),
    dropped: (name, position, total) => m.sort_dropped({ name, position, total }),
    cancelled: (name, position) => m.sort_cancelled({ name, position }),
  };
}
