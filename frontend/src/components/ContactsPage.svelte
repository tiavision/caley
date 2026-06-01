<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import { getUser } from '../lib/user.ts';
  import { apiFetch } from '../lib/api.ts';
  import type { ContactJson, AddressbookJson, ContactsResponse, TypedEntry, AddressEntry } from '../types/calendar.ts';
  import ShareMenu from './ShareMenu.svelte';
  import ContactDetailDialog from './contacts/ContactDetailDialog.svelte';
  import ContactFormDialog from './contacts/ContactFormDialog.svelte';

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
  let abDropdownOpen = false;
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

  function handleWindowClick(e: MouseEvent) {
    if (abDropdownOpen && !(e.target as Element).closest('.ab-dropdown')) {
      abDropdownOpen = false;
    }
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

<svelte:window onclick={handleWindowClick} onkeydown={handleKeydown} />

<div class="contacts-page">
  <div class="contacts-toolbar">
    <h2>{tl('contacts_heading')}</h2>
    <div class="contacts-filters">
      {#if addressbooks.length > 1}
        <div class="ab-dropdown">
          <button
            type="button"
            class="ab-dropdown-btn"
            onclick={() => abDropdownOpen = !abDropdownOpen}
          >
            {abBtnLabel} <span class="dropdown-arrow" aria-hidden="true">▾</span>
          </button>
          {#if abDropdownOpen}
            <ul class="ab-dropdown-list">
              {#each addressbooks as ab (ab.account_id + ab.href)}
                <li>
                  <label>
                    <input
                      type="checkbox"
                      checked={selectedAddressbookHrefs.has(abKey(ab))}
                      onchange={() => toggleAddressbook(abKey(ab))}
                    />
                    {ab.name}
                  </label>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}
      <input
        type="search"
        class="contacts-filter-input"
        bind:value={filterText}
        placeholder={tl('contacts_filterplaceholder')}
      />
    </div>
  </div>

  {#if loading}
    <p class="contacts-status">{tl('weekly_loading')}</p>
  {:else if errorMsg}
    <p class="contacts-status form-error">{errorMsg}</p>
  {:else if filteredContacts.length === 0}
    <p class="contacts-status">{tl('contacts_noContacts')}</p>
  {:else}
    <div class="contacts-groups">
      {#each grouped as group (group.letter)}
        <div class="contact-group">
          <div class="contact-group-letter">{group.letter}</div>
          <div class="contact-cards">
              {#each group.items as contact (contact.account_id + contact.uid + contact.addressbook_href)}
              <!-- svelte-ignore a11y-click-events-have-key-events -->
              <div
                class="contact-card"
                role="button"
                tabindex="0"
                onclick={() => detailDialog?.open(contact)}
                onkeydown={(e) => e.key === 'Enter' && detailDialog?.open(contact)}
              >
                <div class="card-avatar">
                  {#if contact.has_photo}
                    <img class="avatar-photo" src="/api/contact/photo?account_id={encodeURIComponent(contact.account_id)}&uid={encodeURIComponent(contact.uid)}&addressbook_href={encodeURIComponent(contact.addressbook_href)}" alt="" />
                  {:else}
                    {(contact.fn_name[0] ?? '?').toUpperCase()}
                  {/if}
                </div>
                <div class="card-body">
                  <span class="card-name">{contact.fn_name || '—'}</span>
                  {#if contact.emails[0]}
                    <a href="mailto:{contact.emails[0].value}" class="card-link card-email" onclick={(e) => e.stopPropagation()}>{contact.emails[0].value}</a>
                  {/if}
                  {#if contact.phones[0]}
                    <a href="tel:{contact.phones[0].value}" class="card-link card-phone" onclick={(e) => e.stopPropagation()}>{contact.phones[0].value}</a>
                  {/if}
                  {#if contact.org}
                    <span class="card-org">{contact.org}</span>
                  {/if}
                  {#if addressbooks.length > 1}
                    <span class="card-ab">{getAddressbookName(contact.account_id, contact.addressbook_href)}</span>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        </div>
      {/each}
    </div>
  {/if}
</div>

<!-- FAB: new contact -->
<button class="contacts-fab" onclick={() => formDialog?.openNew()} aria-label={tl('contacts_new')}>
  <svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
  </svg>
</button>

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

<style>
  .contacts-page {
    width: 100%;
    box-sizing: border-box;
  }

  /* Toolbar */
  .contacts-toolbar {
    display: flex;
    align-items: center;
    gap: 1rem;
    margin-bottom: 1.25rem;
    flex-wrap: wrap;
  }

  .contacts-toolbar h2 {
    margin: 0;
    flex-shrink: 0;
  }

  .contacts-filters {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
    margin-left: auto;
    padding-right: 1rem;
  }

  .contacts-filter-input {
    margin-bottom: 0;
    font-size: 0.85em;
    width: 16rem;
    max-width: 100%;
  }

  /* Addressbook dropdown — mirrors .cal-dropdown in SearchPage */
  .ab-dropdown {
    position: relative;
  }

  .ab-dropdown-btn {
    opacity: 0.65;
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-size: 0.85em;
    width: auto;
    margin-bottom: 0;
    cursor: pointer;
    background: var(--pico-form-element-background-color);
    border: var(--pico-border-width) solid var(--pico-form-element-border-color);
    box-shadow: none;
    color: var(--pico-form-element-color);
    line-height: var(--pico-line-height);
  }

  .dropdown-arrow {
    font-size: 0.7em;
    opacity: 0.6;
    line-height: 1;
  }

  .ab-dropdown-list {
    position: absolute;
    top: calc(100% + 0.25rem);
    left: 0;
    z-index: 100;
    background: var(--pico-card-background-color);
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    padding: 0.25rem 0;
    margin: 0;
    min-width: 12rem;
    max-height: 16rem;
    overflow-y: auto;
    box-shadow: 0 0.25rem 0.75rem rgba(0, 0, 0, 0.12);
    list-style: none;
  }

  .ab-dropdown-list li {
    list-style: none;
  }

  .ab-dropdown-list label {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    padding: 0.35rem 0.75rem;
    cursor: pointer;
    margin-bottom: 0;
    font-size: 0.9em;
    white-space: nowrap;
  }

  .ab-dropdown-list label:hover {
    background: var(--pico-muted-background-color);
  }

  .ab-dropdown-list input[type="checkbox"] {
    margin-bottom: 0;
    cursor: pointer;
  }

  /* Status messages */
  .contacts-status {
    opacity: 0.6;
    font-size: 0.9em;
    padding: 0.5rem 0;
  }

  /* Contact groups */
  .contacts-groups {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .contact-group-letter {
    font-size: 0.75rem;
    font-weight: 700;
    opacity: 0.45;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    padding: 0.4rem 0 0.25rem;
    border-bottom: 1px solid var(--pico-muted-border-color);
    margin-bottom: 0.5rem;
  }

  /* Card grid */
  .contact-cards {
    display: flex;
    flex-wrap: wrap;
    gap: 0.75rem;
    padding-bottom: 0.75rem;
  }

  /* Individual card */
  .contact-card {
    width: 210px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.4rem;
    padding: 1rem 0.75rem 0.85rem;
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    background: var(--pico-card-background-color);
    cursor: pointer;
    text-align: center;
    transition: border-color 0.15s, box-shadow 0.15s;
  }

  .contact-card:hover {
    border-color: var(--pico-primary);
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.08);
  }

  .card-avatar {
    width: 2.75rem;
    height: 2.75rem;
    border-radius: 50%;
    background: var(--pico-primary);
    color: var(--pico-primary-inverse);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 1.1rem;
    font-weight: 600;
    flex-shrink: 0;
    margin-bottom: 0.15rem;
    overflow: hidden;
  }

  .card-body {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.1rem;
    min-width: 0;
    width: 100%;
  }

  .card-name {
    font-weight: 600;
    font-size: 0.9rem;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }

  .card-link {
    font-size: 0.75rem;
    opacity: 0.7;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
    text-decoration: none;
    color: var(--pico-primary);
  }

  .card-link:hover {
    text-decoration: underline;
    opacity: 1;
  }

  .card-org {
    font-size: 0.75rem;
    opacity: 0.6;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
  }

  .card-ab {
    font-size: 0.7rem;
    opacity: 0.45;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 100%;
    margin-top: 0.15rem;
  }

  /* FAB */
  .contacts-fab {
    position: fixed;
    bottom: 1.5rem;
    right: 1.5rem;
    width: 3.25rem;
    height: 3.25rem;
    border-radius: 50%;
    background: var(--pico-primary);
    color: var(--pico-primary-inverse);
    border: none;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.25);
    z-index: 100;
    margin-bottom: 0;
    padding: 0;
  }

  .contacts-fab:hover {
    background: var(--pico-primary-hover);
  }

  /* Mobile */
  @media (max-width: 480px) {
    .contacts-filter-input {
      width: 100%;
    }

    .contacts-filters {
      width: 100%;
    }

    .contact-card {
      width: calc(50% - 0.375rem);
    }
  }
</style>
