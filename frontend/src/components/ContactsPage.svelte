<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import { getUser } from '../lib/user.ts';
  import { apiFetch } from '../lib/api.ts';
  import type { ContactJson, AddressbookJson, ContactsResponse, TypedEntry, AddressEntry } from '../types/calendar.ts';
  import ShareMenu from './ShareMenu.svelte';
  import ContactDetailDialog from './contacts/ContactDetailDialog.svelte';
  import ContactFormDialog from './contacts/ContactFormDialog.svelte';
  import NavToggle from './NavToggle.svelte';

  let locale = 'en';
  $: tl = makeTl(locale);

  let contacts: ContactJson[] = [];
  let addressbooks: AddressbookJson[] = [];
  let loading = true;
  let errorMsg = '';

  let detailDialog: ContactDetailDialog;
  let formDialog: ContactFormDialog;

  let filterText = '';
  let selectedAddressbookHrefs = new Set<string>();
  let mapUrl = 'https://www.openstreetmap.org/search?query={q}';

  let sseSource: EventSource | null = null;

  onMount(async () => {
    await initI18n();
    locale = getLocale();
    getUser().then(u => { if (u.map_url) mapUrl = u.map_url; });
    await loadContacts();
    connectSSE();
  });

  onDestroy(() => {
    sseSource?.close();
  });

  async function loadContacts() {
    loading = true;
    errorMsg = '';
    try {
      const res = await apiFetch('/api/contacts');
      if (!res.ok) { errorMsg = 'Failed to load contacts.'; return; }
      const data: ContactsResponse = await res.json();
      addressbooks = data.addressbooks;
      contacts = data.contacts;
      if (selectedAddressbookHrefs.size === 0 && addressbooks.length > 0) {
        selectedAddressbookHrefs = new Set(addressbooks.map(ab => abKey(ab)));
      }
    } catch {
      errorMsg = 'Failed to load contacts.';
    } finally {
      loading = false;
    }
  }

  function connectSSE() {
    sseSource = new EventSource('/api/events/stream');
    sseSource.addEventListener('contacts_refetch', () => loadContacts());
    sseSource.addEventListener('contact_updated', (e: MessageEvent) => {
      try {
        const payload: { account_id: string; uid: string; addressbook_href: string; contact?: ContactJson } = JSON.parse(e.data);
        if (payload.contact) {
          const idx = contacts.findIndex(c => c.account_id === payload.account_id && c.uid === payload.uid && c.addressbook_href === payload.addressbook_href);
          if (idx >= 0) {
            contacts[idx] = payload.contact;
            contacts = [...contacts];
            const sel = detailDialog?.getSelected();
            if (sel?.account_id === payload.account_id && sel?.uid === payload.uid) {
              detailDialog?.open(payload.contact);
            }
          } else {
            contacts = sortedInsert(contacts, payload.contact);
          }
        } else {
          loadContacts();
        }
      } catch {}
    });
    sseSource.addEventListener('contact_removed', (e: MessageEvent) => {
      try {
        const payload: { account_id: string; uid: string; addressbook_href: string } = JSON.parse(e.data);
        contacts = contacts.filter(c => !(c.account_id === payload.account_id && c.uid === payload.uid && c.addressbook_href === payload.addressbook_href));
        const sel = detailDialog?.getSelected();
        if (sel?.account_id === payload.account_id && sel?.uid === payload.uid && sel?.addressbook_href === payload.addressbook_href) {
          detailDialog?.close();
        }
      } catch {}
    });
    sseSource.onerror = () => {
      sseSource?.close();
      setTimeout(connectSSE, 5000);
    };
  }

  function sortedInsert(list: ContactJson[], contact: ContactJson): ContactJson[] {
    const result = [...list, contact];
    result.sort((a, b) => a.fn_name.localeCompare(b.fn_name, locale, { sensitivity: 'base' }));
    return result;
  }

  $: filteredContacts = (() => {
    let list = contacts;
    if (addressbooks.length > 1 && selectedAddressbookHrefs.size < addressbooks.length) {
      list = list.filter(c => selectedAddressbookHrefs.has(contactAbKey(c)));
    }
    const q = filterText.trim().toLowerCase();
    if (q) {
      list = list.filter(c =>
        c.fn_name.toLowerCase().includes(q) ||
        c.emails.some(e => e.value.toLowerCase().includes(q)) ||
        c.org.toLowerCase().includes(q) ||
        c.phones.some(p => p.value.toLowerCase().includes(q))
      );
    }
    return list;
  })();

  $: grouped = groupByLetter(filteredContacts);

  function groupByLetter(list: ContactJson[]): { letter: string; items: ContactJson[] }[] {
    const map = new Map<string, ContactJson[]>();
    for (const c of list) {
      const letter = (c.fn_name[0] ?? '#').toUpperCase();
      if (!map.has(letter)) map.set(letter, []);
      map.get(letter)!.push(c);
    }
    return Array.from(map.entries())
      .sort(([a], [b]) => a.localeCompare(b))
      .map(([letter, items]) => ({ letter, items }));
  }

  function getAddressbookName(accountId: string, href: string): string {
    return addressbooks.find(ab => ab.account_id === accountId && ab.href === href)?.name ?? '';
  }

  function abKey(ab: AddressbookJson): string {
    return `${ab.account_id}\0${ab.href}`;
  }

  function contactAbKey(c: ContactJson): string {
    return `${c.account_id}\0${c.addressbook_href}`;
  }

  $: abBtnLabel = (() => {
    const n = selectedAddressbookHrefs.size;
    const total = addressbooks.length;
    return n === total ? tl('contacts_addressbooks') : `${tl('contacts_addressbooks')} (${n}/${total})`;
  })();

  function toggleAddressbook(key: string) {
    if (selectedAddressbookHrefs.has(key)) selectedAddressbookHrefs.delete(key);
    else selectedAddressbookHrefs.add(key);
    selectedAddressbookHrefs = selectedAddressbookHrefs;
  }

  function handleKeydown(e: KeyboardEvent) {
    if (formDialog?.isOpen()) return;
    const target = e.target as HTMLElement;
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || target.isContentEditable) return;
    if (e.key === 'c' && !detailDialog?.isOpen()) {
      e.preventDefault();
      formDialog?.openNew();
    } else if (e.key === 'e' && detailDialog?.isOpen()) {
      const sel = detailDialog?.getSelected();
      if (sel) { e.preventDefault(); detailDialog?.close(); formDialog?.openEdit(sel); }
    }
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div data-layout="stack" data-gap="md" data-pad="sm">
  <div data-layout="row" data-gap="sm">
    <NavToggle />
    <input
      type="search"
      data-grow
      style="--tm-grow-basis: 12rem"
      bind:value={filterText}
      placeholder={tl('contacts_filterplaceholder')}
      aria-label={tl('contacts_filterplaceholder')}
    />
    {#if addressbooks.length > 1}
      <button type="button" popovertarget="ab-menu" data-variant="outline">{abBtnLabel}</button>
      <div popover id="ab-menu" data-menu>
        {#each addressbooks as ab (ab.account_id + ab.href)}
          <label>
            <input
              type="checkbox"
              checked={selectedAddressbookHrefs.has(abKey(ab))}
              onchange={() => toggleAddressbook(abKey(ab))}
            />
            {ab.name}
          </label>
        {/each}
      </div>
    {/if}
  </div>

  {#if loading}
    <p aria-busy="true">{tl('weekly_loading')}</p>
  {:else if errorMsg}
    <div data-callout data-tone="danger" role="alert">{errorMsg}</div>
  {:else if filteredContacts.length === 0}
    <p data-muted>{tl('contacts_noContacts')}</p>
  {:else}
    {#each grouped as group (group.letter)}
      <section data-layout="stack" data-gap="sm">
        <h2 data-caption>{group.letter}</h2>
        <div data-layout="grid" data-gap="sm" style="--tm-min: 10rem">
          {#each group.items as contact (contact.account_id + contact.uid + contact.addressbook_href)}
            <article data-layout="stack" data-gap="xs" data-align="center">
              <span data-avatar>
                {#if contact.has_photo}
                  <img src="/api/contact/photo?account_id={encodeURIComponent(contact.account_id)}&uid={encodeURIComponent(contact.uid)}&addressbook_href={encodeURIComponent(contact.addressbook_href)}" alt="" />
                {:else}
                  {(contact.fn_name[0] ?? '?').toUpperCase()}
                {/if}
              </span>
              <button
                type="button"
                data-stretch
                data-variant="link"
                data-truncate
                commandfor="contact-detail"
                command="show-modal"
                onclick={() => detailDialog?.open(contact)}
              ><strong>{contact.fn_name || '—'}</strong></button>
              {#if contact.emails[0]}
                <small data-truncate><a href="mailto:{contact.emails[0].value}">{contact.emails[0].value}</a></small>
              {/if}
              {#if contact.phones[0]}
                <small data-truncate><a href="tel:{contact.phones[0].value}">{contact.phones[0].value}</a></small>
              {/if}
              {#if contact.org}
                <small data-muted data-truncate>{contact.org}</small>
              {/if}
              {#if addressbooks.length > 1}
                <small data-muted data-truncate>{getAddressbookName(contact.account_id, contact.addressbook_href)}</small>
              {/if}
            </article>
          {/each}
        </div>
      </section>
    {/each}
  {/if}
</div>

<button
  type="button"
  data-fab
  data-icon="plus"
  aria-label={tl('contacts_new')}
  commandfor="contact-form"
  command="show-modal"
  onclick={() => formDialog?.openNew()}
></button>

<!-- Detail dialog -->
<ContactDetailDialog
  bind:this={detailDialog}
  {tl}
  {locale}
  {addressbooks}
  {mapUrl}
  onedit={(c) => { detailDialog?.close(); formDialog?.openEdit(c); }}
  ondelete={() => {}}
/>

<!-- Form dialog -->
<ContactFormDialog
  bind:this={formDialog}
  {tl}
  {addressbooks}
  onsaved={async (saved) => { await detailDialog?.open(saved); }}
/>
