// Islands of the mail pages (mail/index.html, mail/view/index.html): the HTML body iframe, the column resize
// handles, the reader header and the page-hidden signal. They measure layout or own a foreign document; none
// writes sinter state or sinter-managed nodes (they report through DOM events).

const overlayQuery = '(max-width: 1023px)';

/**
 * `<mail-frame>`: an HTML mail body in a sandboxed iframe, `data-sn-prop="{html, isolated, version, label}"`.
 * `isolated` (decrypted mail with inline data: images) leaves out allow-same-origin. In the narrow layout the frame
 * grows to its content, since the whole reader scrolls there. A new `version` reloads it (after allowing an image
 * origin). `height` fixes the height instead (the compose dialog's forward preview).
 */
class MailFrame extends HTMLElement {
  #props = { html: '', isolated: false, version: 0, label: '', height: '' };
  #frame: HTMLIFrameElement | null = null;
  #shown = '';
  #queued = false;
  #observer: ResizeObserver | null = null;
  #media = window.matchMedia(overlayQuery);
  #onMedia = () => this.#fit();

  get html() { return this.#props.html; }
  set html(v: string) { this.#set('html', v ?? ''); }
  get isolated() { return this.#props.isolated; }
  set isolated(v: boolean) { this.#set('isolated', !!v); }
  get version() { return this.#props.version; }
  set version(v: number) { this.#set('version', v ?? 0); }
  get label() { return this.#props.label; }
  set label(v: string) { this.#set('label', v ?? ''); }
  get height() { return this.#props.height; }
  set height(v: string) { this.#set('height', v ?? ''); }

  #set<K extends keyof MailFrame['_props']>(key: K, value: MailFrame['_props'][K]) {
    this.#props[key] = value;
    if (this.#queued) return;
    this.#queued = true;
    // sinter sets the props one by one: render once, after all of them.
    queueMicrotask(() => { this.#queued = false; this.#render(); });
  }

  /** for the setter's types only */
  declare _props: { html: string; isolated: boolean; version: number; label: string; height: string };

  connectedCallback() {
    this.#media.addEventListener('change', this.#onMedia);
    this.#render();
  }

  disconnectedCallback() {
    this.#media.removeEventListener('change', this.#onMedia);
    this.#observer?.disconnect();
  }

  #render() {
    if (!this.isConnected) return;
    const { html, isolated, version, label, height } = this.#props;
    const shown = `${version}|${isolated}|${height}|${html}`;
    if (this.#frame && shown === this.#shown) {
      this.#frame.title = label;
      return;
    }
    this.#shown = shown;
    this.#observer?.disconnect();
    const frame = document.createElement('iframe');
    frame.setAttribute('sandbox', isolated
      ? 'allow-popups allow-popups-to-escape-sandbox'
      : 'allow-popups allow-same-origin allow-popups-to-escape-sandbox');
    frame.title = label;
    if (height) frame.style.height = height;
    frame.srcdoc = html;
    frame.addEventListener('load', () => this.#fit());
    this.#frame?.remove();
    this.#frame = frame;
    this.append(frame);
  }

  #fit() {
    const frame = this.#frame;
    if (!frame || this.#props.height) return;
    this.#observer?.disconnect();
    if (!this.#media.matches) {
      frame.style.height = '';
      frame.removeAttribute('scrolling');
      return;
    }
    if (!this.#props.isolated) frame.setAttribute('scrolling', 'no');
    try {
      const doc = frame.contentDocument;
      if (!doc) return;
      const setHeight = () => {
        const h = Math.max(doc.documentElement.scrollHeight, doc.documentElement.offsetHeight, doc.documentElement.clientHeight,
          doc.body?.scrollHeight ?? 0, doc.body?.offsetHeight ?? 0, doc.body?.clientHeight ?? 0);
        if (h > 0) frame.style.height = `${Math.ceil(h) + 2}px`;
      };
      setHeight();
      this.#observer = new ResizeObserver(setHeight);
      this.#observer.observe(doc.documentElement);
      if (doc.body) this.#observer.observe(doc.body);
    } catch { /* an isolated frame is cross-origin and cannot be measured */ }
  }
}
customElements.define('mail-frame', MailFrame);

/**
 * `<mail-split>`: a column resize handle. Dragging it reports `split-resize` with the new width in `detail`
 * (`data-sn-prop="{width}"` is the width when the drag starts); arrow keys are the page's (keyWidth helper).
 */
class MailSplit extends HTMLElement {
  #width = 0;
  get width() { return this.#width; }
  set width(v: number) { this.#width = v ?? 0; }

  connectedCallback() {
    this.addEventListener('pointerdown', this.#down);
  }

  disconnectedCallback() {
    this.removeEventListener('pointerdown', this.#down);
  }

  #down = (e: PointerEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const startX = e.clientX;
    const start = this.width;
    this.setPointerCapture(e.pointerId);
    this.toggleAttribute('data-dragging', true);
    const move = (ev: PointerEvent) => {
      this.dispatchEvent(new CustomEvent('split-resize', { bubbles: true, detail: start + ev.clientX - startX }));
    };
    const up = () => {
      this.removeEventListener('pointermove', move);
      this.removeEventListener('pointerup', up);
      this.removeEventListener('pointercancel', up);
      this.toggleAttribute('data-dragging', false);
    };
    this.addEventListener('pointermove', move);
    this.addEventListener('pointerup', up);
    this.addEventListener('pointercancel', up);
  };
}
customElements.define('mail-split', MailSplit);

/**
 * `<mail-header-fit>`: the reader's subject row. When the subject and the controls do not fit on one line, it sets
 * its own `stacked` attribute: controls on the first line, the whole subject below (mail.scss).
 */
class MailHeaderFit extends HTMLElement {
  #observer = new ResizeObserver(() => this.#measure());
  #mutations = new MutationObserver(() => this.#measure());

  connectedCallback() {
    this.#observer.observe(this);
    this.#mutations.observe(this, { subtree: true, childList: true, characterData: true });
    requestAnimationFrame(() => this.#measure());
  }

  disconnectedCallback() {
    this.#observer.disconnect();
    this.#mutations.disconnect();
  }

  #measure() {
    const line = this.querySelector<HTMLElement>('.detail-subject-line');
    const controls = this.querySelector<HTMLElement>('.detail-controls');
    if (!line || !controls) return;
    const style = getComputedStyle(this);
    const gap = parseFloat(style.columnGap || style.gap) || 0;
    const stacked = intrinsicWidth(line) + intrinsicWidth(controls) + gap > this.clientWidth;
    if (stacked !== this.hasAttribute('stacked')) this.toggleAttribute('stacked', stacked);
  }
}

function intrinsicWidth(element: HTMLElement): number {
  const clone = element.cloneNode(true) as HTMLElement;
  Object.assign(clone.style, { position: 'fixed', left: '-10000px', top: '0', width: 'max-content', maxWidth: 'none', flex: 'none', whiteSpace: 'nowrap', visibility: 'hidden', pointerEvents: 'none' });
  document.body.appendChild(clone);
  const width = clone.getBoundingClientRect().width;
  clone.remove();
  return width;
}
customElements.define('mail-header-fit', MailHeaderFit);

/**
 * `<mail-hidden-watch hidden>`: reports `page-hidden` when the page is hidden or left, so the reader can drop decrypted
 * content (MailApp.clearPgpContentWhenHidden).
 */
class MailHiddenWatch extends HTMLElement {
  #fire = () => {
    if (document.hidden) this.dispatchEvent(new CustomEvent('page-hidden', { bubbles: true }));
  };
  #leave = () => this.dispatchEvent(new CustomEvent('page-hidden', { bubbles: true }));

  connectedCallback() {
    document.addEventListener('visibilitychange', this.#fire);
    window.addEventListener('pagehide', this.#leave);
  }

  disconnectedCallback() {
    document.removeEventListener('visibilitychange', this.#fire);
    window.removeEventListener('pagehide', this.#leave);
  }
}
customElements.define('mail-hidden-watch', MailHiddenWatch);
