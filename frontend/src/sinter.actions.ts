// sinter local actions: imperative work pages run with `data-sn-call="ns.fn(args)"` or
// `<sn-resource call="ns.fn(args)">`. One namespace per concern, each in its own module under src/actions/.
// Functions receive the arguments, then sinter's CallContext; they return data and never touch the DOM
// or sinter state (`data-sn-then` puts the result where it belongs).
export * as notify from './actions/notify.ts';
export * as share from './actions/share.ts';
export * as maps from './actions/maps.ts';
// Settings: ICS import dialog (partials/import-dialog.html).
export * as ics from './actions/ics.ts';
