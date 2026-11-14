// The way back after a sign-in is the server's (backend/src/landing.rs): `/` redirects someone signed in to
// the page they wanted (`?next=`, kept in the session) or the app page they used last, else the calendar.

/** The login page, to come back to this page after the sign-in. */
export function loginUrl(): string {
  return `/?${new URLSearchParams({ next: `${location.pathname}${location.search}${location.hash}` })}`;
}
