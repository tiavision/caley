// Caley's app shell (native/): the web APIs Android's WebView lacks, given to the server's pages so Caley's
// frontend uses them as in a browser, without knowing about the app. Injected at document start by
// ShellBridge.kt, which answers over the `caleyShell` channel (androidx.webkit's web message listener):
// - `Notification` (Android notifications; a tap opens the app and fires the notification's click),
// - `navigator.share` (Android's share sheet, a file or text),
// - saving `<a download>` links to blob:/data: URLs the page made (to Downloads; links to the server's URLs
//   are Android's download manager's, ShellBridge.kt's download listener),
// - WebAuthn (`navigator.credentials.create/get` with `publicKey`, the PRF extension included): the app is
//   the authenticator (Authenticator.kt, keys in the Android Keystore behind the screen lock), since the
//   WebView offers no passkeys for a server the app cannot name in advance. Android 11+ (`__WEBAUTHN__`).
// `__PERMISSION__` is the notification permission when the page starts (ShellBridge.kt writes it in), since
// `Notification.permission` must be answered at once.
(() => {
  const shell = window.caleyShell;
  if (!shell || location.hostname === 'tauri.localhost' || window.__caleyShell) return;
  window.__caleyShell = true;

  let permission = '__PERMISSION__';
  // this document's notifications, apart from the ones a page loaded later sent
  const page = Math.random().toString(36).slice(2);
  let seq = 0;
  const replies = new Map();
  const notifications = new Map();

  const send = (message) => shell.postMessage(JSON.stringify(message));
  const call = (type, args = {}) =>
    new Promise((resolve, reject) => {
      const id = ++seq;
      replies.set(id, { resolve, reject });
      send({ ...args, id, type });
    });
  const dataUrl = (blob) =>
    new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => resolve(reader.result);
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(blob);
    });

  shell.addEventListener('message', (event) => {
    const message = JSON.parse(event.data);
    if (message.reply) {
      const reply = replies.get(message.reply);
      replies.delete(message.reply);
      if (!reply) return;
      if (message.error) reply.reject(new DOMException(message.error, message.name || 'NotAllowedError'));
      else reply.resolve(message.value);
    } else if (message.click && message.page === page) {
      notifications.get(message.click)?.dispatchEvent(new Event('click'));
    } else if (message.permission) {
      permission = message.permission;
    }
  });
  // the channel ShellBridge.kt answers on and sends notification clicks to
  send({ type: 'hello' });

  class Notification extends EventTarget {
    static get permission() {
      return permission;
    }

    static async requestPermission(callback) {
      permission = await call('permission');
      callback?.(permission);
      return permission;
    }

    #id = ++seq;

    constructor(title, options = {}) {
      super();
      this.title = String(title);
      this.body = options.body ?? '';
      this.tag = options.tag ?? '';
      this.data = options.data ?? null;
      this.onclick = this.onclose = this.onshow = this.onerror = null;
      this.addEventListener('click', (e) => this.onclick?.call(this, e));
      if (permission !== 'granted') {
        queueMicrotask(() => this.onerror?.call(this, new Event('error')));
        return;
      }
      notifications.set(this.#id, this);
      send({ type: 'notify', id: this.#id, page, title: this.title, body: this.body, tag: this.tag });
    }

    close() {
      notifications.delete(this.#id);
      send({ type: 'close', id: this.#id, page });
    }
  }
  window.Notification = Notification;

  navigator.share = async (data = {}) => {
    const file = data.files?.[0];
    await call('share', {
      title: data.title ?? '',
      text: [data.text, data.url].filter(Boolean).join('\n'),
      file: file ? { name: file.name, type: file.type || 'application/octet-stream', data: await dataUrl(file) } : null,
    });
  };
  navigator.canShare = (data = {}) => !data.files || data.files.length <= 1;

  // a download of data the page made (an export, a decrypted attachment): read here, saved by the app. The
  // blob behind a blob: URL is kept from its creation, since the page's CSP (connect-src 'self') does not let
  // it be fetched; a data: URL is its data already.
  const blobs = new Map();
  const createObjectURL = URL.createObjectURL;
  const revokeObjectURL = URL.revokeObjectURL;
  URL.createObjectURL = (object) => {
    const url = createObjectURL.call(URL, object);
    if (object instanceof Blob) blobs.set(url, object);
    return url;
  };
  URL.revokeObjectURL = (url) => {
    blobs.delete(url);
    revokeObjectURL.call(URL, url);
  };
  window.addEventListener('click', (event) => {
    const link = event.target instanceof Element ? event.target.closest('a[download]') : null;
    if (!link || event.defaultPrevented) return;
    const blob = blobs.get(link.href);
    const data = blob ? null : link.href.startsWith('data:') ? link.href : undefined;
    if (data === undefined) return;
    event.preventDefault();
    const name = link.download || 'download';
    Promise.resolve(data ?? dataUrl(blob))
      .then((data) =>
        call('save', { name, mime: blob?.type || /^data:([^;,]*)/.exec(data)[1] || 'application/octet-stream', data }),
      )
      .catch(() => {});
  });

  if (__WEBAUTHN__) {
    // BufferSource ⇄ base64url, the form the app takes and gives
    const b64u = (source) => {
      const bytes = source instanceof ArrayBuffer ? new Uint8Array(source) : new Uint8Array(source.buffer, source.byteOffset, source.byteLength);
      let s = '';
      for (const b of bytes) s += String.fromCharCode(b);
      return btoa(s).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
    };
    const bytes = (s) => Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0)).buffer;

    class PublicKeyCredential {
      constructor(result, response) {
        this.id = result.id;
        this.rawId = bytes(result.id);
        this.type = 'public-key';
        this.authenticatorAttachment = 'platform';
        this.response = response;
        this.extensions = result.prf ? { prf: { enabled: true, results: { first: bytes(result.prf) } } } : result.prfEnabled ? { prf: { enabled: true } } : {};
      }
      getClientExtensionResults() {
        return this.extensions;
      }
      toJSON() {
        return { id: this.id, rawId: this.id, type: this.type };
      }
      static async isUserVerifyingPlatformAuthenticatorAvailable() {
        return true;
      }
      static async isConditionalMediationAvailable() {
        return false;
      }
      static async getClientCapabilities() {
        return { userVerifyingPlatformAuthenticator: true, passkeyPlatformAuthenticator: true, 'extension:prf': true };
      }
    }
    window.PublicKeyCredential = PublicKeyCredential;

    const prfInput = (extensions) => {
      const first = extensions?.prf?.eval?.first;
      return first ? b64u(first) : null;
    };
    const container = navigator.credentials;
    const create = container?.create?.bind(container);
    const get = container?.get?.bind(container);
    const credentials = {
      async create(options = {}) {
        const pk = options.publicKey;
        if (!pk) return create ? create(options) : null;
        const result = await call('webauthn.create', {
          challenge: b64u(pk.challenge),
          rpId: pk.rp?.id ?? location.hostname,
          userId: b64u(pk.user.id),
          userName: pk.user.name ?? '',
          algorithms: (pk.pubKeyCredParams ?? []).map((p) => p.alg),
          exclude: (pk.excludeCredentials ?? []).map((c) => b64u(c.id)),
          prf: prfInput(pk.extensions),
          wantsPrf: !!pk.extensions?.prf,
        });
        return new PublicKeyCredential(result, {
          clientDataJSON: bytes(result.clientDataJSON),
          attestationObject: bytes(result.attestationObject),
          getTransports: () => ['internal'],
          getAuthenticatorData: () => bytes(result.authenticatorData),
          getPublicKeyAlgorithm: () => -7,
        });
      },
      async get(options = {}) {
        const pk = options.publicKey;
        if (!pk) return get ? get(options) : null;
        const result = await call('webauthn.get', {
          challenge: b64u(pk.challenge),
          rpId: pk.rpId ?? location.hostname,
          allow: (pk.allowCredentials ?? []).map((c) => b64u(c.id)),
          prf: prfInput(pk.extensions),
        });
        return new PublicKeyCredential(result, {
          clientDataJSON: bytes(result.clientDataJSON),
          authenticatorData: bytes(result.authenticatorData),
          signature: bytes(result.signature),
          userHandle: result.userHandle ? bytes(result.userHandle) : null,
        });
      },
      store: (credential) => container?.store?.(credential),
      preventSilentAccess: () => container?.preventSilentAccess?.() ?? Promise.resolve(),
    };
    Object.defineProperty(navigator, 'credentials', { value: credentials, configurable: true });
  }
})();
