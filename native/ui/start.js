// Runs before the picker paints (a blocking script in its head): when the app starts with a server used
// before, that server opens at once and the picker never shows (html[data-opening] hides it meanwhile).
// Coming back to choose (`?pick`, from Back or the menu) shows it. picker.js keeps the list (`caley:servers`).

// Opens a server's pages. On Android the app is told first which server it is (ShellBridge.kt: only that
// origin's pages get the bridge) and the page goes there once it has answered, so the server's first page
// finds it bound; without an answer in time it goes anyway (the server's pages then have no bridge).
function openServer(origin) {
  const shell = window.caleyShell;
  let gone = false;
  const go = () => {
    if (gone) return;
    gone = true;
    location.assign(`${origin}/`);
  };
  if (!shell) return go();
  shell.addEventListener('message', go);
  shell.postMessage(JSON.stringify({ type: 'server', origin }));
  setTimeout(go, 2000);
}

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
  openServer(last.origin);
})();
