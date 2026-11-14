// sinter helpers: pure functions templates call by name (`eventTimeLabel(ev)`). The page module imports only what
// a page uses. Messages (`m.nav_mail()`) are sinter's own, from messages/{locale}.json (project.inlang); TS code
// imports `m` from '@tiaforge-ui/sinter/messages'. sinter's built-in date(), time(), datetime(), relative(),
// number() and bytes() need no export here; they format with $locale, the language sinter picked.
// Add a page's formatting helpers here (or re-export them from a module next to the page); keep them pure.

// pagePath('/calendar/'): a page URL in this page's language, for navigate() (the build localizes only the links in the markup)
export { pagePath } from './lib/locale-path.ts';

export {
  partstatKey,
  localDateIso,
  eventTimeLabel,
  formatEventTime,
  formatEventTimes,
  formatRecurrence,
  formatAlarm,
} from './lib/format.ts';

export { contactShare, eventShare } from './lib/share.ts';

export {
  addressbookKey,
  sameContact,
  filterContacts,
  groupByLetter,
  contactKey,
  addressbookName,
  contactInitial,
  contactPhotoUrl,
  addressQuery,
  addressLines,
  newDraft,
  editDraft,
  draftBody,
  emptyEntry,
  emptyAddress,
  addressbooksLabel,
} from './lib/contacts.ts';

export { geoUrl } from './actions/maps.ts';

// Event components (src/components/event-detail.html, event-form.html), search and calendar pages.
export {
  calendarKey,
  eventKey,
  calendarName,
  partstatTone,
  partstatLabel,
  attendeeLabel,
  eventDateLong,
  weekdays,
  newEventDraft,
  editEventDraft,
  todayIso,
  newAlarm,
  newAttendee,
  halfHourAfter,
  eventFormBody,
  intervalUnit,
  calendarGroups,
  summarySuggestions,
  applySummarySuggestion,
  attendeeSuggestions,
  sameEvent,
  calendarsLabel,
  newCalendarDraft,
  newCalendarBody,
} from './lib/events.ts';

// Thread dialog (src/components/thread-dialog.html).
export { threadRows, messageUrl } from './lib/thread.ts';

// ICS import dialog (src/components/import-dialog.html).
export { calendarValue, importEventDate, importDraft, importSummary } from './lib/import.ts';

// Settings page: the CalDAV account dialog, calendar toggles, encryption tab errors.
export { caldavDraft, caldavBody, withCalendarActive, keyImportError, enrollErrorText } from './lib/settings.ts';
// Errors of calls that may load openpgp: "reload the page" for a stale one (encryption tab, mail reader, compose).
export { loadErrorOr } from './lib/errors.ts';

// Mail pages (mail/index.html, mail/view/index.html, src/components/mail-*.html).
export {
  currentView, folderView, mailboxView, sameView, folderName, viewTitle, badgeCount, mailboxKey, sidebarSections, mailTitle,
  toggled, refOf, isRef, urlRef, filterActive, markRange, neighbor, archiveMailbox, moveTargets, bulkOf,
  withFlag, flagBody, moveBody, bulkFlagsBody, bulkMoveBody, bulkBody, inTrash, emptyTrashDays, validTrashDays, clampWidth,
  keyWidth, isSentMailbox, messageDateTime, shownAttachments, attachmentUrl, attachmentCount, pgpBadgeLabel, isIconBadge,
  bodyOrigins, matching, isThreadCurrent, composeDraft, composeTitle, recipientToken, withRecipient, contactAddresses,
  recipientHits, senderHits, senderLabel, canSign, canEncrypt, signs, newAccount, editAccount, discovered, accountBody, accountUi,
  identityBody, folderDraft, folderBody, subscribed, canPreviewAttachment, folderKind, htmlAddsValue,
  isIcsAttachment, formatDate, humanSize, textSegments, messageKey, senderName,
  // page state (src/lib/mail.ts "Page state") and the keys without a control of their own
  bodyOf, readerOn, noReader, inMailbox, noSelection, unmarked, cleared, openedAt, rowClick, opensRow, afterBulk,
  noSearch, searchActive, addableSender, senderAdded, listStatus, noView, noDrag, canDrop, markedRead, sharedDraft,
  pageKey, readerScroller, scrollTopFor, siblingRow,
  // the page's URL (src/lib/mail.ts "The mail page's URL")
  urlView, shownView, viewHref, urlLags, noSearchBox, mailSearch,
} from './lib/mail.ts';

// Calendar page.
export {
  userTz, addDays, mondayOf, monthOf, addMonths, weekRangeText, dayLabel, dayCollapsed, eventsOn, weekMove,
  weekPositions, weekColumnDate, farStep, inWeek, weekdayNames, monthGridHeader, monthNames, pickerYears, monthTitle,
  pickerMonth, cellEvents, swipeStep, monthLabel, monthCells, monthRange, weekDates, hasDay,
} from './lib/calendar.ts';
export { threadOfEmail } from './lib/thread.ts';

// Focus and scroll targets for handlers (typed, no casts).
export { elementIn, ancestor } from './lib/dom.ts';
