// The server picker: the Caley servers this app knows (localStorage, this page's origin only), adding one
// after the shell has checked it answers as Caley (`probe_server`, src-tauri/src/lib.rs), and opening one:
// the webview goes to the server's own pages, which work there as in a browser. Back from the server's
// first page (Android, MainActivity.kt) and the desktop menu's "Switch server" come back here as `/?pick`.

const text = {
  en: {
    saved: 'Your servers',
    add: 'Add a server',
    address: 'Server address',
    hint: 'The address you open Caley at in a browser.',
    connect: 'Connect',
    checking: 'Connecting…',
    remove: 'Remove {origin}',
    invalid_url: 'That is not a web address.',
    unreachable: 'No answer from that address.',
    not_caley: 'There is no Caley server at that address.',
  },
  de: {
    saved: 'Deine Server',
    add: 'Server hinzufügen',
    address: 'Serveradresse',
    hint: 'Die Adresse, unter der du Caley im Browser öffnest.',
    connect: 'Verbinden',
    checking: 'Verbinde…',
    remove: '{origin} entfernen',
    invalid_url: 'Das ist keine Webadresse.',
    unreachable: 'Keine Antwort von dieser Adresse.',
    not_caley: 'Unter dieser Adresse läuft kein Caley-Server.',
  },
};
const lang = navigator.language.toLowerCase().startsWith('de') ? 'de' : 'en';
const t = (key, params = {}) => text[lang][key].replace(/\{(\w+)\}/g, (_, name) => params[name] ?? '');

// { origin }[], most recently opened first
const SERVERS = 'caley:servers';

function servers() {
  try {
    const list = JSON.parse(localStorage.getItem(SERVERS) ?? '[]');
    return Array.isArray(list) ? list.filter((s) => typeof s?.origin === 'string') : [];
  } catch {
    return [];
  }
}

function save(list) {
  localStorage.setItem(SERVERS, JSON.stringify(list));
}

function open(origin) {
  save([{ origin }, ...servers().filter((s) => s.origin !== origin)]);
  // the server sends the visitor on to their language and to the sign-in or the page they used last
  location.assign(`${origin}/`);
}

// the saved servers as rows to open (host, then the whole address) and the form to add one: open on its
// own while there is none, folded under "Add a server" once there is
function render() {
  const list = servers();
  document.getElementById('saved').hidden = list.length === 0;
  const adding = document.getElementById('adding');
  adding.classList.toggle('only', list.length === 0);
  if (list.length === 0) adding.open = true;
  const ul = document.getElementById('servers');
  ul.replaceChildren(
    ...list.map(({ origin }) => {
      const li = document.createElement('li');
      const openButton = document.createElement('button');
      openButton.type = 'button';
      openButton.className = 'open';
      const host = document.createElement('span');
      host.className = 'host';
      host.textContent = new URL(origin).host;
      const address = document.createElement('span');
      address.className = 'origin';
      address.textContent = origin;
      openButton.append(host, address);
      openButton.addEventListener('click', () => open(origin));
      const remove = document.createElement('button');
      remove.type = 'button';
      remove.className = 'remove';
      remove.textContent = '×';
      remove.setAttribute('aria-label', t('remove', { origin }));
      remove.addEventListener('click', () => {
        save(servers().filter((s) => s.origin !== origin));
        render();
      });
      li.append(openButton, remove);
      return li;
    }),
  );
}

async function connect(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const button = form.querySelector('button[type=submit]');
  const error = document.getElementById('error');
  const address = form.address.value.trim();
  if (!address) return form.address.focus();
  error.textContent = '';
  button.disabled = true;
  button.textContent = t('checking');
  try {
    const { origin } = await window.__TAURI__.core.invoke('probe_server', { address });
    open(origin);
  } catch (e) {
    error.textContent = t(text.en[e?.error] ? e.error : 'unreachable');
    button.disabled = false;
    button.textContent = t('connect');
  }
}

document.documentElement.lang = lang;
for (const el of document.querySelectorAll('[data-t]')) el.textContent = t(el.dataset.t);
document.getElementById('add').addEventListener('submit', connect);
document.getElementById('adding').addEventListener('toggle', (e) => {
  if (e.newState === 'open' && !e.currentTarget.classList.contains('only')) document.getElementById('address').focus();
});
render();

// a start of the app opens the server used last before this page paints (start.js); coming back to choose
// (`?pick`) shows the list
// shown again from the back/forward cache: the buttons as they were before a connect, the page visible
window.addEventListener('pageshow', (e) => {
  if (!e.persisted) return;
  delete document.documentElement.dataset.opening;
  const button = document.querySelector('#add button[type=submit]');
  button.disabled = false;
  button.textContent = t('connect');
  render();
});
