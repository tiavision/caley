<script lang="ts">
  import { tick } from 'svelte';
  import { makeTl } from '../../lib/i18n.ts';
  import type { AddressbookJson, ContactJson, AddressEntry } from '../../types/calendar.ts';
  import ShareMenu from '../ShareMenu.svelte';

  export let tl: ReturnType<typeof makeTl>;
  export let locale: string = 'en';
  export let addressbooks: AddressbookJson[] = [];
  export let mapUrl: string = 'https://www.openstreetmap.org/search?query={q}';
  export let onedit: (contact: ContactJson) => void = () => {};
  export let ondelete: (contact: ContactJson) => void = () => {};

  let dialog: HTMLDialogElement;
  let selectedContact: ContactJson | null = null;

  export async function open(c: ContactJson) {
    selectedContact = c;
    await tick();
    dialog?.showModal();
  }

  export function close() {
    selectedContact = null;
    dialog?.close();
  }

  export function isOpen(): boolean {
    return dialog?.open ?? false;
  }

  export function getSelected(): ContactJson | null {
    return selectedContact;
  }

  function addressQuery(addr: AddressEntry): string {
    return [addr.street, addr.city, addr.region, addr.postal, addr.country].filter(Boolean).join(', ');
  }

  function openMap(event: MouseEvent, query: string) {
    event.preventDefault();
    const encoded = encodeURIComponent(query);
    let appOpened = false;
    const onBlur = () => { appOpened = true; };
    const onVisibility = () => { if (document.hidden) appOpened = true; };
    window.addEventListener('blur', onBlur, { once: true });
    document.addEventListener('visibilitychange', onVisibility, { once: true });
    window.location.href = `geo:0,0?q=${encoded}`;
    setTimeout(() => {
      window.removeEventListener('blur', onBlur);
      document.removeEventListener('visibilitychange', onVisibility);
      if (!appOpened) window.open(mapUrl.replace('{q}', encoded), '_blank');
    }, 500);
  }

  function getAddressbookName(accountId: string, href: string): string {
    return addressbooks.find(ab => ab.account_id === accountId && ab.href === href)?.name ?? '';
  }

  async function deleteContact(c: ContactJson) {
    if (!confirm(tl('contacts_deleteConfirm'))) return;
    close();
    try {
      await fetch('/api/contact', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ account_id: c.account_id, uid: c.uid, addressbook_href: c.addressbook_href }),
      });
    } catch {}
    ondelete(c);
  }
</script>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={dialog}
  id="contact-detail"
  onclick={(e) => { if (e.target === e.currentTarget) close(); }}
  onclose={close}
  oncancel={(e) => { e.preventDefault(); close(); }}
>
  <article>
    <header>
      <h3>{selectedContact?.fn_name || ''}</h3>
      <div class="header-actions">
        {#if selectedContact}
          <ShareMenu contact={selectedContact} {locale} />
          <button
            class="edit-btn"
            title={tl('contacts_edit')}
            onclick={() => onedit(selectedContact!)}
          >&#9998;</button>
        {/if}
        <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={close}></button>
      </div>
    </header>

    {#if selectedContact}
      <div class="contact-detail-body">
        <div class="detail-avatar-wrap">
          <div class="detail-avatar">
            {#if selectedContact.has_photo}
              <img class="avatar-photo" src="/api/contact/photo?account_id={encodeURIComponent(selectedContact.account_id)}&uid={encodeURIComponent(selectedContact.uid)}&addressbook_href={encodeURIComponent(selectedContact.addressbook_href)}" alt="" />
            {:else}
              {(selectedContact.fn_name[0] ?? '?').toUpperCase()}
            {/if}
          </div>
        </div>

        {#if selectedContact.org}
          <div class="detail-row">
            <span class="detail-label">{tl('contacts_org')}</span>
            <span>{selectedContact.org}</span>
          </div>
        {/if}

        {#if selectedContact.emails.length > 0}
          <div class="detail-row">
            <span class="detail-label">{tl('contacts_email')}</span>
            <div class="detail-links">
              {#each selectedContact.emails as entry}
                <div class="typed-entry">
                  <span class="type-chip">{tl(`contacts_type_${entry.type_label}`)}</span>
                  <a href="mailto:{entry.value}">{entry.value}</a>
                </div>
              {/each}
            </div>
          </div>
        {/if}

        {#if selectedContact.phones.length > 0}
          <div class="detail-row">
            <span class="detail-label">{tl('contacts_phone')}</span>
            <div class="detail-links">
              {#each selectedContact.phones as entry}
                <div class="typed-entry">
                  <span class="type-chip">{tl(`contacts_type_${entry.type_label}`)}</span>
                  <a href="tel:{entry.value}">{entry.value}</a>
                </div>
              {/each}
            </div>
          </div>
        {/if}

        {#if selectedContact.addresses.length > 0}
          <div class="detail-row">
            <span class="detail-label">{tl('contacts_address')}</span>
            <div class="detail-links">
              {#each selectedContact.addresses as addr}
                <div class="typed-entry">
                  <span class="type-chip">{tl(`contacts_type_${addr.type_label}`)}</span>
                  <a href="geo:0,0?q={encodeURIComponent(addressQuery(addr))}" onclick={(e) => openMap(e, addressQuery(addr))} class="address-text" rel="noopener noreferrer">
                    {#if addr.street}{addr.street}<br />{/if}
                    {#if addr.city || addr.region || addr.postal}{addr.city}{addr.region ? (addr.city ? ', ' : '') + addr.region : ''}{addr.postal ? ' ' + addr.postal : ''}<br />{/if}
                    {#if addr.country}{addr.country}{/if}
                  </a>
                </div>
              {/each}
            </div>
          </div>
        {/if}

        {#if addressbooks.length > 1}
          <div class="detail-row">
            <span class="detail-label">{tl('contacts_addressbook')}</span>
            <span>{getAddressbookName(selectedContact.account_id, selectedContact.addressbook_href)}</span>
          </div>
        {/if}

        <div class="detail-actions">
          <button class="secondary outline" onclick={() => deleteContact(selectedContact!)}>{tl('contacts_delete')}</button>
        </div>
      </div>
    {/if}
  </article>
</dialog>

<style>
  .header-actions {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }

  .edit-btn {
    background: none;
    border: none;
    cursor: pointer;
    font-size: 1rem;
    color: var(--pico-muted-color);
    padding: 0.25rem;
    line-height: 1;
    transition: color 0.15s;
    margin-bottom: 0;
    width: auto;
  }

  .edit-btn:hover {
    color: var(--pico-primary);
  }

  .detail-avatar-wrap {
    display: flex;
    justify-content: center;
    margin-bottom: 0.25rem;
  }

  .detail-avatar {
    width: 4rem;
    height: 4rem;
    border-radius: 50%;
    background: var(--pico-primary);
    color: var(--pico-primary-inverse);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.6rem;
    font-weight: 600;
    overflow: hidden;
  }

  .avatar-photo {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }

  .contact-detail-body {
    margin-top: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.75rem;
  }

  .detail-row {
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
  }

  .detail-label {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--pico-muted-color, #666);
    font-weight: 600;
  }

  .detail-links {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
  }

  .detail-links a {
    font-size: 0.95rem;
    color: var(--pico-primary);
    text-decoration: none;
  }

  .detail-links a:hover {
    text-decoration: underline;
  }

  .typed-entry {
    display: flex;
    align-items: baseline;
    gap: 0.4rem;
  }

  .type-chip {
    font-size: 0.7rem;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--pico-muted-color, #666);
    background: var(--pico-muted-background-color);
    border-radius: 0.25rem;
    padding: 0.05rem 0.35rem;
    flex-shrink: 0;
    line-height: 1.4;
  }

  .address-text {
    font-size: 0.9rem;
    line-height: 1.5;
  }

  .detail-actions {
    display: flex;
    gap: 0.5rem;
    margin-top: 0.5rem;
  }

  .detail-actions button {
    margin-bottom: 0;
  }
</style>
