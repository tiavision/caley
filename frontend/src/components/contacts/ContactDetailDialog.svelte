<script lang="ts">
  import { tick } from 'svelte';
  import { makeTl } from '../../lib/i18n.ts';
  import { apiFetch } from '../../lib/api.ts';
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
    if (!dialog?.open) dialog?.showModal();
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
      await apiFetch('/api/contact', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ account_id: c.account_id, uid: c.uid, addressbook_href: c.addressbook_href }),
      });
    } catch {}
    ondelete(c);
  }
</script>

<dialog
  bind:this={dialog}
  id="contact-detail"
  closedby="any"
  onclose={close}
>
  <header>
    <h2>{selectedContact?.fn_name || ''}</h2>
    <div data-layout="row" data-gap="xs" data-wrap="none">
      {#if selectedContact}
        <ShareMenu contact={selectedContact} {locale} />
        <button
          type="button"
          data-icon="pencil"
          data-variant="ghost"
          aria-label={tl('contacts_edit')}
          data-tooltip
          onclick={() => onedit(selectedContact!)}
        ></button>
      {/if}
      <button type="button" commandfor="contact-detail" command="close" aria-label={tl('weekly_aria_close')}></button>
    </div>
  </header>

  {#if selectedContact}
    <span data-avatar data-self="center" style="--tm-avatar-size: 4rem">
      {#if selectedContact.has_photo}
        <img src="/api/contact/photo?account_id={encodeURIComponent(selectedContact.account_id)}&uid={encodeURIComponent(selectedContact.uid)}&addressbook_href={encodeURIComponent(selectedContact.addressbook_href)}" alt="" />
      {:else}
        {(selectedContact.fn_name[0] ?? '?').toUpperCase()}
      {/if}
    </span>

    <dl data-pairs>
      {#if selectedContact.org}
        <dt>{tl('contacts_org')}</dt>
        <dd>{selectedContact.org}</dd>
      {/if}

      {#if selectedContact.emails.length > 0}
        <dt>{tl('contacts_email')}</dt>
        <dd data-layout="stack" data-gap="xs">
          {#each selectedContact.emails as entry}
            <span data-layout="row" data-gap="sm" data-wrap="none">
              <span data-badge>{tl(`contacts_type_${entry.type_label}`)}</span>
              <a href="mailto:{entry.value}" data-truncate>{entry.value}</a>
            </span>
          {/each}
        </dd>
      {/if}

      {#if selectedContact.phones.length > 0}
        <dt>{tl('contacts_phone')}</dt>
        <dd data-layout="stack" data-gap="xs">
          {#each selectedContact.phones as entry}
            <span data-layout="row" data-gap="sm" data-wrap="none">
              <span data-badge>{tl(`contacts_type_${entry.type_label}`)}</span>
              <a href="tel:{entry.value}">{entry.value}</a>
            </span>
          {/each}
        </dd>
      {/if}

      {#if selectedContact.addresses.length > 0}
        <dt>{tl('contacts_address')}</dt>
        <dd data-layout="stack" data-gap="sm">
          {#each selectedContact.addresses as addr}
            <span data-layout="row" data-gap="sm" data-wrap="none" data-align="start">
              <span data-badge>{tl(`contacts_type_${addr.type_label}`)}</span>
              <a href="geo:0,0?q={encodeURIComponent(addressQuery(addr))}" onclick={(e) => openMap(e, addressQuery(addr))} rel="noopener noreferrer">
                {#if addr.street}{addr.street}<br />{/if}
                {#if addr.city || addr.region || addr.postal}{addr.city}{addr.region ? (addr.city ? ', ' : '') + addr.region : ''}{addr.postal ? ' ' + addr.postal : ''}<br />{/if}
                {#if addr.country}{addr.country}{/if}
              </a>
            </span>
          {/each}
        </dd>
      {/if}

      {#if addressbooks.length > 1}
        <dt>{tl('contacts_addressbook')}</dt>
        <dd>{getAddressbookName(selectedContact.account_id, selectedContact.addressbook_href)}</dd>
      {/if}
    </dl>

    <footer>
      <button type="button" data-variant="outline" data-tone="danger" data-icon="trash-2" onclick={() => deleteContact(selectedContact!)}>{tl('contacts_delete')}</button>
    </footer>
  {/if}
</dialog>
