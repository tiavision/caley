// Settings page islands: navigation after the account is deleted (sinter expressions cannot navigate).
// The delete-account button emits this with the server's answer ({redirect}).
document.addEventListener('account-deleted', (e) => {
  location.href = (e as CustomEvent<{ redirect?: string } | null>).detail?.redirect || '/calendar/';
});
