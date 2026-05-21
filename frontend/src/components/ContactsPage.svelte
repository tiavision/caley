<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import type { ContactJson, AddressbookJson, ContactsResponse } from '../types/calendar.ts';

  let locale = 'en';
  $: tl = makeTl(locale);

  let contacts: ContactJson[] = [];
  let addressbooks: AddressbookJson[] = [];
  let loading = true;
  let errorMsg = '';

  // Detail panel
  let selectedContact: ContactJson | null = null;

  // Form state
  let showForm = false;
  let formAddressbookHref = '';
  let formUid: string | null = null;
  let formName = '';
  let formEmails: string[] = [''];
  let formPhones: string[] = [''];
  let formOrg = '';
  let formSaving = false;
  let formError = '';

  let sseSource: EventSource | null = null;

  onMount(async () => {
    await initI18n();
    locale = getLocale();
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
      const res = await fetch('/api/contacts');
      if (!res.ok) { errorMsg = 'Failed to load contacts.'; return; }
      const data: ContactsResponse = await res.json();
      addressbooks = data.addressbooks;
      contacts = data.contacts;
      if (!formAddressbookHref && addressbooks.length > 0) {
        formAddressbookHref = addressbooks[0].href;
      }
    } catch {
      errorMsg = 'Failed to load contacts.';
    } finally {
      loading = false;
    }
  }

  function connectSSE() {
    sseSource = new EventSource('/api/events/stream');

    sseSource.addEventListener('contacts_refetch', () => {
      loadContacts();
    });

    sseSource.addEventListener('contact_updated', (e: MessageEvent) => {
      try {
        const payload: { uid: string; addressbook_href: string; contact?: ContactJson } = JSON.parse(e.data);
        if (payload.contact) {
          const idx = contacts.findIndex(c => c.uid === payload.uid && c.addressbook_href === payload.addressbook_href);
          if (idx >= 0) {
            contacts[idx] = payload.contact;
            contacts = [...contacts];
            if (selectedContact?.uid === payload.uid) selectedContact = payload.contact;
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
        const payload: { uid: string; addressbook_href: string } = JSON.parse(e.data);
        contacts = contacts.filter(c => !(c.uid === payload.uid && c.addressbook_href === payload.addressbook_href));
        if (selectedContact?.uid === payload.uid) selectedContact = null;
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

  // Alphabetical grouping
  $: grouped = groupByLetter(contacts);

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

  function selectContact(c: ContactJson) {
    selectedContact = c;
    showForm = false;
  }

  function openNewForm() {
    formUid = null;
    formName = '';
    formEmails = [''];
    formPhones = [''];
    formOrg = '';
    formAddressbookHref = addressbooks[0]?.href ?? '';
    formError = '';
    showForm = true;
    selectedContact = null;
  }

  function openEditForm(c: ContactJson) {
    formUid = c.uid;
    formName = c.fn_name;
    formEmails = c.emails.length > 0 ? [...c.emails] : [''];
    formPhones = c.phones.length > 0 ? [...c.phones] : [''];
    formOrg = c.org;
    formAddressbookHref = c.addressbook_href;
    formError = '';
    showForm = true;
  }

  function closeForm() {
    showForm = false;
  }

  async function saveContact(e: SubmitEvent) {
    e.preventDefault();
    formSaving = true;
    formError = '';
    const emails = formEmails.filter(s => s.trim());
    const phones = formPhones.filter(s => s.trim());
    try {
      const res = await fetch('/api/contact', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          uid: formUid ?? undefined,
          addressbook_href: formAddressbookHref,
          fn_name: formName.trim(),
          emails,
          phones,
          org: formOrg.trim(),
        }),
      });
      if (!res.ok) {
        formError = tl('contacts_error');
      } else {
        showForm = false;
        const saved: ContactJson = await res.json();
        selectedContact = saved;
      }
    } catch {
      formError = tl('contacts_error');
    } finally {
      formSaving = false;
    }
  }

  async function deleteContact(c: ContactJson) {
    if (!confirm(tl('contacts_deleteConfirm'))) return;
    try {
      await fetch('/api/contact', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ uid: c.uid, addressbook_href: c.addressbook_href }),
      });
      if (selectedContact?.uid === c.uid) selectedContact = null;
    } catch {}
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      if (showForm) { showForm = false; }
      else { selectedContact = null; }
    }
  }
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="contacts-layout">
  <!-- List panel -->
  <div class="contacts-list-panel">
    <div class="contacts-list-header">
      <h2>{tl('contacts_heading')}</h2>
    </div>

    {#if loading}
      <p class="contacts-loading">{tl('weekly_loading')}</p>
    {:else if errorMsg}
      <p class="form-error">{errorMsg}</p>
    {:else if contacts.length === 0}
      <p class="contacts-empty">{tl('contacts_noContacts')}</p>
    {:else}
      <div class="contacts-list">
        {#each grouped as group (group.letter)}
          <div class="contact-group">
            <div class="contact-group-letter" aria-hidden="true">{group.letter}</div>
            {#each group.items as contact (contact.uid + contact.addressbook_href)}
              <button
                class="contact-row"
                class:selected={selectedContact?.uid === contact.uid && selectedContact?.addressbook_href === contact.addressbook_href}
                on:click={() => selectContact(contact)}
              >
                <div class="contact-avatar">{(contact.fn_name[0] ?? '?').toUpperCase()}</div>
                <div class="contact-row-body">
                  <span class="contact-name">{contact.fn_name || '—'}</span>
                  {#if contact.emails[0]}
                    <span class="contact-email">{contact.emails[0]}</span>
                  {/if}
                </div>
              </button>
            {/each}
          </div>
        {/each}
      </div>
    {/if}
  </div>

  <!-- Detail / form panel -->
  <div class="contacts-detail-panel">
    {#if showForm}
      <form class="contact-form" on:submit={saveContact}>
        <h3>{formUid ? tl('contacts_edit') : tl('contacts_new')}</h3>

        {#if formError}
          <p class="form-error">{formError}</p>
        {/if}

        <label>
          {tl('contacts_name')}
          <input type="text" bind:value={formName} required />
        </label>

        {#each formEmails as _, i}
          <label>
            {tl('contacts_email')} {formEmails.length > 1 ? i + 1 : ''}
            <div class="field-row">
              <input type="email" bind:value={formEmails[i]} />
              {#if formEmails.length > 1}
                <button type="button" class="remove-btn" on:click={() => { formEmails = formEmails.filter((_, j) => j !== i); }}>✕</button>
              {/if}
            </div>
          </label>
        {/each}
        <button type="button" class="add-field-btn" on:click={() => { formEmails = [...formEmails, '']; }}>
          + {tl('contacts_addEmail')}
        </button>

        {#each formPhones as _, i}
          <label>
            {tl('contacts_phone')} {formPhones.length > 1 ? i + 1 : ''}
            <div class="field-row">
              <input type="tel" bind:value={formPhones[i]} />
              {#if formPhones.length > 1}
                <button type="button" class="remove-btn" on:click={() => { formPhones = formPhones.filter((_, j) => j !== i); }}>✕</button>
              {/if}
            </div>
          </label>
        {/each}
        <button type="button" class="add-field-btn" on:click={() => { formPhones = [...formPhones, '']; }}>
          + {tl('contacts_addPhone')}
        </button>

        <label>
          {tl('contacts_org')}
          <input type="text" bind:value={formOrg} />
        </label>

        {#if addressbooks.length > 1}
          <label>
            {tl('contacts_addressbook')}
            <select bind:value={formAddressbookHref}>
              {#each addressbooks as ab}
                <option value={ab.href}>{ab.name}</option>
              {/each}
            </select>
          </label>
        {/if}

        <div class="form-actions">
          <button type="submit" aria-busy={formSaving} disabled={formSaving}>{tl('contacts_save')}</button>
          <button type="button" class="secondary" on:click={closeForm}>{tl('contacts_cancel')}</button>
        </div>
      </form>

    {:else if selectedContact}
      <div class="contact-detail">
        <div class="contact-detail-avatar">{(selectedContact.fn_name[0] ?? '?').toUpperCase()}</div>
        <h3 class="contact-detail-name">{selectedContact.fn_name}</h3>

        {#if selectedContact.org}
          <p class="contact-detail-org">{selectedContact.org}</p>
        {/if}

        {#if selectedContact.emails.length > 0}
          <div class="contact-detail-section">
            <span class="contact-detail-label">{tl('contacts_email')}</span>
            {#each selectedContact.emails as email}
              <a href="mailto:{email}" class="contact-detail-value">{email}</a>
            {/each}
          </div>
        {/if}

        {#if selectedContact.phones.length > 0}
          <div class="contact-detail-section">
            <span class="contact-detail-label">{tl('contacts_phone')}</span>
            {#each selectedContact.phones as phone}
              <a href="tel:{phone}" class="contact-detail-value">{phone}</a>
            {/each}
          </div>
        {/if}

        <div class="contact-detail-actions">
          <button on:click={() => openEditForm(selectedContact!)}>{tl('contacts_edit')}</button>
          <button class="secondary outline" on:click={() => deleteContact(selectedContact!)}>{tl('contacts_delete')}</button>
        </div>
      </div>

    {:else}
      <div class="contacts-placeholder">
        <p>Select a contact to view details.</p>
      </div>
    {/if}
  </div>
</div>

<!-- FAB: new contact -->
{#if !showForm}
  <button class="contacts-fab" on:click={openNewForm} aria-label={tl('contacts_new')}>
    <svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
      <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
    </svg>
  </button>
{/if}

<style>
  .contacts-layout {
    display: grid;
    grid-template-columns: 280px 1fr;
    gap: 0;
    min-height: calc(100vh - 8rem);
    min-height: calc(100dvh - 8rem);
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    overflow: hidden;
  }

  @media (max-width: 600px) {
    .contacts-layout {
      grid-template-columns: 1fr;
    }
    .contacts-detail-panel {
      display: none;
    }
    .contacts-layout:has(.contact-detail),
    .contacts-layout:has(.contact-form) {
      grid-template-columns: 1fr;
    }
  }

  .contacts-list-panel {
    border-right: 1px solid var(--pico-muted-border-color);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  .contacts-list-header {
    padding: 0.75rem 1rem 0.5rem;
    border-bottom: 1px solid var(--pico-muted-border-color);
    display: flex;
    align-items: baseline;
  }

  .contacts-list-header h2 {
    margin: 0;
    font-size: 1.1rem;
  }

  .contacts-list {
    flex: 1;
    overflow-y: auto;
  }

  .contacts-loading,
  .contacts-empty {
    padding: 1rem;
    opacity: 0.6;
    font-size: 0.9em;
  }

  .contact-group-letter {
    padding: 0.3rem 1rem 0.1rem;
    font-size: 0.75rem;
    font-weight: 700;
    opacity: 0.5;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    border-top: 1px solid var(--pico-muted-border-color);
  }

  .contact-group:first-child .contact-group-letter {
    border-top: none;
  }

  .contact-row {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    width: 100%;
    padding: 0.5rem 1rem;
    border: none;
    background: transparent;
    cursor: pointer;
    text-align: left;
    font-size: inherit;
    color: inherit;
  }

  .contact-row:hover {
    background: var(--pico-card-background-color);
  }

  .contact-row.selected {
    background: var(--pico-primary-background);
    color: var(--pico-primary-inverse);
  }

  .contact-avatar {
    width: 2rem;
    height: 2rem;
    border-radius: 50%;
    background: var(--pico-primary);
    color: var(--pico-primary-inverse);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 0.85rem;
    font-weight: 600;
    flex-shrink: 0;
  }

  .contact-row.selected .contact-avatar {
    background: var(--pico-primary-inverse);
    color: var(--pico-primary);
  }

  .contact-row-body {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .contact-name {
    font-weight: 500;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .contact-email {
    font-size: 0.8em;
    opacity: 0.7;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .contacts-detail-panel {
    padding: 1.5rem;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  .contact-detail {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 0.5rem;
    max-width: 480px;
    margin: 0 auto;
    width: 100%;
  }

  .contact-detail-avatar {
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
    margin-bottom: 0.5rem;
  }

  .contact-detail-name {
    margin: 0;
    font-size: 1.3rem;
    text-align: center;
  }

  .contact-detail-org {
    margin: 0;
    opacity: 0.65;
    font-size: 0.9em;
    text-align: center;
  }

  .contact-detail-section {
    width: 100%;
    margin-top: 0.75rem;
    display: flex;
    flex-direction: column;
    gap: 0.15rem;
  }

  .contact-detail-label {
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    opacity: 0.55;
    font-weight: 600;
  }

  .contact-detail-value {
    font-size: 0.95rem;
    color: var(--pico-primary);
  }

  .contact-detail-actions {
    display: flex;
    gap: 0.5rem;
    margin-top: 1rem;
    width: 100%;
  }

  .contact-detail-actions button {
    flex: 1;
    margin-bottom: 0;
  }

  .contacts-placeholder {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 1;
    opacity: 0.4;
    font-size: 0.9em;
  }

  /* Form styles */
  .contact-form {
    max-width: 480px;
    width: 100%;
  }

  .contact-form h3 {
    margin-top: 0;
  }

  .field-row {
    display: flex;
    gap: 0.4rem;
    align-items: center;
  }

  .field-row input {
    flex: 1;
    margin-bottom: 0;
  }

  .remove-btn {
    padding: 0.25rem 0.5rem;
    background: transparent;
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    cursor: pointer;
    font-size: 0.8rem;
    color: var(--pico-muted-color);
    margin-bottom: 0;
    width: auto;
    flex-shrink: 0;
  }

  .add-field-btn {
    background: transparent;
    border: none;
    padding: 0.1rem 0;
    font-size: 0.8rem;
    color: var(--pico-primary);
    cursor: pointer;
    text-align: left;
    margin-bottom: 0.75rem;
    width: auto;
  }

  .form-actions {
    display: flex;
    gap: 0.5rem;
    margin-top: 1rem;
  }

  .form-actions button {
    flex: 1;
    margin-bottom: 0;
  }

  .form-error {
    color: var(--pico-del-color, red);
    font-size: 0.875em;
    margin: 0 0 0.5rem;
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
    box-shadow: 0 2px 8px rgba(0,0,0,0.25);
    z-index: 100;
    margin-bottom: 0;
    padding: 0;
  }

  .contacts-fab:hover {
    background: var(--pico-primary-hover);
  }
</style>
