// Runs before the picker paints (a blocking script in its head): when the app starts with a server used
// before, that server opens at once and the picker never shows (html[data-opening] hides it meanwhile).
// Coming back to choose (`?pick`, from Back or the menu) shows it. picker.js keeps the list (`caley:servers`).
(() => {
  if (new URLSearchParams(location.search).has('pick')) return;
  let last;
  try {
    [last] = JSON.parse(localStorage.getItem('caley:servers') ?? '[]');
  } catch {
    return;
  }
  if (typeof last?.origin !== 'string') return;
  document.documentElement.dataset.opening = '';
  location.assign(`${last.origin}/`);
})();
