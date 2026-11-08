// sinter helpers: pure functions templates call by name (`eventTimeLabel(ev)`), plus the `m` namespace of
// paraglide messages (`m.nav_mail()`, `m.login_error()`). The page module imports only what a page uses.
// sinter's built-in date(), time(), datetime(), relative(), number() and bytes() need no export here; they
// format with $locale, which src/sinter.config.ts connects to paraglide (src/lib/i18n.ts).
// Add a page's formatting helpers here (or re-export them from a module next to the page); keep them pure.
export * as m from './paraglide/messages/_index.js';

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
  shortcutKey,
} from './lib/contacts.ts';

export { geoUrl } from './actions/maps.ts';

// Event partials (partials/event-detail.html, event-form.html), search and calendar pages.
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
  chosenCalendars,
  searchQuery,
  sameSearch,
  searchSources,
} from './lib/events.ts';

// ICS import dialog (partials/import-dialog.html).
export { calendarValue, importEventDate } from './lib/import.ts';

// Mail pages (mail/index.html, mail/view/index.html, partials/mail-*.html).
export {
  currentView, folderView, mailboxView, sameView, folderName, viewTitle, badgeCount, mailboxKey, sidebarSections, mailTitle,
  toggled, refOf, isRef, urlRef, filterActive, markRange, neighbor, archiveMailbox, archiveTargets, moveTargets, bulkOf,
  withFlag, flagBody, moveBody, inTrash, emptyTrashDays, validTrashDays, mailShortcut, rowKey, readerScroll, clampWidth,
  keyWidth, isSentMailbox, messageDateTime, shownAttachments, attachmentUrl, attachmentCount, pgpBadgeLabel, isIconBadge,
  bodyOrigins, matching, isThreadCurrent, composeDraft, composeTitle, recipientToken, withRecipient, contactAddresses,
  recipientHits, senderHits, senderLabel, canSign, canEncrypt, signs, newAccount, editAccount, discovered, accountBody,
  identityBody, folderDraft, folderBody, subscribed, canPreviewAttachment, folderKind, htmlAddsValue,
  isIcsAttachment, formatDate, humanSize, textSegments, messageKey, senderName,
} from './lib/mail.ts';
