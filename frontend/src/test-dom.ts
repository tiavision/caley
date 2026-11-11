// What jsdom lacks for the pages' dialogs, as browsers do it. Imported by the page and component tests.

// Modal dialogs: open/close set `open` and fire `toggle` (and `close`).
if (!HTMLDialogElement.prototype.showModal) {
  HTMLDialogElement.prototype.showModal = function () { if (this.open) return; this.open = true; this.dispatchEvent(new Event('toggle')); };
  HTMLDialogElement.prototype.close = function () { if (!this.open) return; this.open = false; this.dispatchEvent(new Event('toggle')); this.dispatchEvent(new Event('close')); };
}
if (!HTMLElement.prototype.hidePopover) HTMLElement.prototype.hidePopover = function () {};

// Command invokers: a click on a [commandfor] button runs its command after the click's handlers, as the
// browser's default action does (show-modal on an open dialog and close on a closed one do nothing).
document.addEventListener('click', (e) => {
  const button = (e.target as Element).closest?.('button[commandfor]');
  if (!button || e.defaultPrevented) return;
  const dialog = document.getElementById(button.getAttribute('commandfor')!);
  if (!(dialog instanceof HTMLDialogElement)) return;
  const command = button.getAttribute('command');
  queueMicrotask(() => (command === 'show-modal' ? dialog.showModal() : command === 'close' ? dialog.close() : undefined));
});
