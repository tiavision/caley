<script lang="ts">
  import { onMount, onDestroy, tick } from 'svelte';
  import { initI18n, getLocale, makeTl } from '../lib/i18n.ts';
  import type { ContactJson, AddressbookJson, ContactsResponse, TypedEntry, AddressEntry } from '../types/calendar.ts';

  let locale = 'en';
  $: tl = makeTl(locale);

  let contacts: ContactJson[] = [];
  let addressbooks: AddressbookJson[] = [];
  let loading = true;
  let errorMsg = '';

  let selectedContact: ContactJson | null = null;
  let detailDialog: HTMLDialogElement;

  let filterText = '';
  let selectedAddressbookHrefs = new Set<string>();
  let abDropdownOpen = false;

  let formDialog: HTMLDialogElement;
  const emptyEmail = (): TypedEntry => ({ value: '', type_label: 'other' });
  const emptyPhone = (): TypedEntry => ({ value: '', type_label: 'other' });
  const emptyAddress = (): AddressEntry => ({ street: '', city: '', region: '', postal: '', country: '', type_label: 'home' });

  function addressQuery(addr: AddressEntry) {
    return [addr.street, addr.city, addr.region, addr.postal, addr.country].filter(Boolean).join(', ');
  }

  function openMap(event: MouseEvent, query: string) {
    event.preventDefault();
    const encoded = encodeURIComponent(query);
    window.location.href = `geo:0,0?q=${encoded}`;
    setTimeout(() => {
      if (!document.hidden) {
        window.open(`https://www.openstreetmap.org/search?query=${encoded}`, '_blank');
      }
    }, 800);
  }

  let formUid: string | null = null;
  let formName = '';
  let formNameInput: HTMLInputElement;
  let formEmails: TypedEntry[] = [emptyEmail()];
  let formPhones: TypedEntry[] = [emptyPhone()];
  let formAddresses: AddressEntry[] = [];
  let formOrg = '';
  let formAddressbookHref = '';
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
      if (selectedAddressbookHrefs.size === 0 && addressbooks.length > 0) {
        selectedAddressbookHrefs = new Set(addressbooks.map(ab => ab.href));
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
        if (selectedContact?.uid === payload.uid && selectedContact?.addressbook_href === payload.addressbook_href) {
          closeDetail();
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
      list = list.filter(c => selectedAddressbookHrefs.has(c.addressbook_href));
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

  function getAddressbookName(href: string): string {
    return addressbooks.find(ab => ab.href === href)?.name ?? '';
  }

  $: abBtnLabel = (() => {
    const n = selectedAddressbookHrefs.size;
    const total = addressbooks.length;
    return n === total ? tl('contacts_addressbooks') : `${tl('contacts_addressbooks')} (${n}/${total})`;
  })();

  function toggleAddressbook(href: string) {
    if (selectedAddressbookHrefs.has(href)) selectedAddressbookHrefs.delete(href);
    else selectedAddressbookHrefs.add(href);
    selectedAddressbookHrefs = selectedAddressbookHrefs;
  }

  function handleWindowClick(e: MouseEvent) {
    if (abDropdownOpen && !(e.target as Element).closest('.ab-dropdown')) {
      abDropdownOpen = false;
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if (formDialog?.open) return;
    const target = e.target as HTMLElement;
    if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || target.isContentEditable) return;
    if (e.key === 'c' && !detailDialog?.open) {
      e.preventDefault();
      openNewForm();
    } else if (e.key === 'e' && detailDialog?.open && selectedContact) {
      e.preventDefault();
      openEditForm(selectedContact);
    }
  }

  async function openDetail(c: ContactJson) {
    selectedContact = c;
    await tick();
    detailDialog?.showModal();
  }

  function closeDetail() {
    selectedContact = null;
    detailDialog?.close();
  }

  async function openNewForm() {
    formUid = null;
    formName = '';
    formEmails = [emptyEmail()];
    formPhones = [emptyPhone()];
    formAddresses = [];
    formOrg = '';
    formAddressbookHref = addressbooks[0]?.href ?? '';
    formError = '';
    await tick();
    formDialog?.showModal();
    formDialog?.querySelector('article')?.scrollTo(0, 0);
    formNameInput?.select();
  }

  async function openEditForm(c: ContactJson) {
    if (detailDialog?.open) {
      selectedContact = null;
      detailDialog.close();
    }
    formUid = c.uid;
    formName = c.fn_name;
    formEmails = c.emails.length > 0 ? c.emails.map(e => ({ ...e })) : [emptyEmail()];
    formPhones = c.phones.length > 0 ? c.phones.map(p => ({ ...p })) : [emptyPhone()];
    formAddresses = c.addresses.map(a => ({ ...a }));
    formOrg = c.org;
    formAddressbookHref = c.addressbook_href;
    formError = '';
    await tick();
    formDialog?.showModal();
    formDialog?.querySelector('article')?.scrollTo(0, 0);
    formNameInput?.select();
  }

  function closeForm() {
    formDialog?.close();
  }

  async function saveContact(e: SubmitEvent) {
    e.preventDefault();
    formSaving = true;
    formError = '';
    const emails = formEmails.filter(e => e.value.trim());
    const phones = formPhones.filter(p => p.value.trim());
    const addresses = formAddresses.filter(a => a.street.trim() || a.city.trim() || a.country.trim());
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
          addresses,
          org: formOrg.trim(),
        }),
      });
      if (!res.ok) {
        formError = tl('contacts_error');
      } else {
        const saved: ContactJson = await res.json();
        closeForm();
        await tick();
        await openDetail(saved);
      }
    } catch {
      formError = tl('contacts_error');
    } finally {
      formSaving = false;
    }
  }

  async function deleteContact(c: ContactJson) {
    if (!confirm(tl('contacts_deleteConfirm'))) return;
    closeDetail();
    try {
      await fetch('/api/contact', {
        method: 'DELETE',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ uid: c.uid, addressbook_href: c.addressbook_href }),
      });
    } catch {}
  }
</script>

<svelte:window onclick={handleWindowClick} onkeydown={handleKeydown} />

<div class="contacts-page">
  <div class="contacts-toolbar">
    <a href="/" class="home-btn" aria-label={tl('nav_calendar')}>
      <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <rect x="3" y="4" width="18" height="18" rx="2" ry="2"/><line x1="16" y1="2" x2="16" y2="6"/><line x1="8" y1="2" x2="8" y2="6"/><line x1="3" y1="10" x2="21" y2="10"/>
      </svg>
    </a>
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
              {#each addressbooks as ab (ab.href)}
                <li>
                  <label>
                    <input
                      type="checkbox"
                      checked={selectedAddressbookHrefs.has(ab.href)}
                      onchange={() => toggleAddressbook(ab.href)}
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
            {#each group.items as contact (contact.uid + contact.addressbook_href)}
              <!-- svelte-ignore a11y-click-events-have-key-events -->
              <div
                class="contact-card"
                role="button"
                tabindex="0"
                onclick={() => openDetail(contact)}
                onkeydown={(e) => e.key === 'Enter' && openDetail(contact)}
              >
                <div class="card-avatar">
                  {#if contact.has_photo}
                    <img class="avatar-photo" src="/api/contact/photo?uid={encodeURIComponent(contact.uid)}&addressbook_href={encodeURIComponent(contact.addressbook_href)}" alt="" />
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
                    <span class="card-ab">{getAddressbookName(contact.addressbook_href)}</span>
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
<button class="contacts-fab" onclick={openNewForm} aria-label={tl('contacts_new')}>
  <svg xmlns="http://www.w3.org/2000/svg" width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
    <line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/>
  </svg>
</button>

<!-- Detail dialog -->
<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={detailDialog}
  id="contact-detail"
  onclick={(e) => { if (e.target === e.currentTarget) closeDetail(); }}
  onclose={closeDetail}
  oncancel={(e) => { e.preventDefault(); closeDetail(); }}
>
  <article>
    <header>
      <h3>{selectedContact?.fn_name || ''}</h3>
      <div class="header-actions">
        {#if selectedContact}
          <button
            class="edit-btn"
            title={tl('contacts_edit')}
            onclick={() => openEditForm(selectedContact!)}
          >&#9998;</button>
        {/if}
        <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={closeDetail}></button>
      </div>
    </header>

    {#if selectedContact}
      <div class="contact-detail-body">
        <div class="detail-avatar-wrap">
          <div class="detail-avatar">
            {#if selectedContact.has_photo}
              <img class="avatar-photo" src="/api/contact/photo?uid={encodeURIComponent(selectedContact.uid)}&addressbook_href={encodeURIComponent(selectedContact.addressbook_href)}" alt="" />
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
            <span>{getAddressbookName(selectedContact.addressbook_href)}</span>
          </div>
        {/if}

        <div class="detail-actions">
          <button class="secondary outline" onclick={() => deleteContact(selectedContact!)}>{tl('contacts_delete')}</button>
        </div>
      </div>
    {/if}
  </article>
</dialog>

<!-- Form dialog -->
<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={formDialog}
  id="contact-form"
  onclick={(e) => { if (e.target === e.currentTarget) closeForm(); }}
  oncancel={(e) => { e.preventDefault(); closeForm(); }}
>
  <article>
    <header>
      <h3>{formUid ? tl('contacts_edit') : tl('contacts_new')}</h3>
      <div class="header-actions">
        <button class="close-btn" aria-label={tl('weekly_aria_close')} onclick={closeForm}></button>
      </div>
    </header>

    <form onsubmit={saveContact}>
      {#if formError}
        <p class="form-error">{formError}</p>
      {/if}

      <label>
        {tl('contacts_name')}
        <input type="text" bind:value={formName} bind:this={formNameInput} required />
      </label>

      <fieldset class="contact-field-section">
        <legend>{tl('contacts_email')}</legend>
        <ul class="contact-field-list">
          {#each formEmails as entry, i}
            <li class="contact-field-row">
              <input type="email" bind:value={formEmails[i].value} />
              <select bind:value={formEmails[i].type_label} class="type-select">
                <option value="home">{tl('contacts_type_home')}</option>
                <option value="work">{tl('contacts_type_work')}</option>
                <option value="other">{tl('contacts_type_other')}</option>
              </select>
              <button type="button" class="remove-btn" onclick={() => { formEmails = formEmails.filter((_, j) => j !== i); }}>×</button>
            </li>
          {/each}
        </ul>
        <button type="button" class="outline small" onclick={() => { formEmails = [...formEmails, emptyEmail()]; }}>
          {tl('contacts_addEmail')}
        </button>
      </fieldset>

      <fieldset class="contact-field-section">
        <legend>{tl('contacts_phone')}</legend>
        <ul class="contact-field-list">
          {#each formPhones as entry, i}
            <li class="contact-field-row">
              <input type="tel" bind:value={formPhones[i].value} />
              <select bind:value={formPhones[i].type_label} class="type-select">
                <option value="home">{tl('contacts_type_home')}</option>
                <option value="work">{tl('contacts_type_work')}</option>
                <option value="mobile">{tl('contacts_type_mobile')}</option>
                <option value="fax">{tl('contacts_type_fax')}</option>
                <option value="pager">{tl('contacts_type_pager')}</option>
                <option value="other">{tl('contacts_type_other')}</option>
              </select>
              <button type="button" class="remove-btn" onclick={() => { formPhones = formPhones.filter((_, j) => j !== i); }}>×</button>
            </li>
          {/each}
        </ul>
        <button type="button" class="outline small" onclick={() => { formPhones = [...formPhones, emptyPhone()]; }}>
          {tl('contacts_addPhone')}
        </button>
      </fieldset>

      <fieldset class="contact-field-section">
        <legend>{tl('contacts_address')}</legend>
        <ul class="contact-addr-list">
          {#each formAddresses as addr, i}
            <li class="contact-addr-item">
              <div class="contact-addr-header">
                <span>{tl('contacts_address')}{formAddresses.length > 1 ? ` ${i + 1}` : ''}</span>
                <button type="button" class="remove-btn" onclick={() => { formAddresses = formAddresses.filter((_, j) => j !== i); }}>×</button>
              </div>
              <label>
                {tl('contacts_street')}
                <input type="text" bind:value={formAddresses[i].street} />
              </label>
              <div class="addr-row">
                <label class="addr-city">
                  {tl('contacts_city')}
                  <input type="text" bind:value={formAddresses[i].city} />
                </label>
                <label class="addr-region">
                  {tl('contacts_region')}
                  <input type="text" bind:value={formAddresses[i].region} />
                </label>
                <label class="addr-postal">
                  {tl('contacts_postal')}
                  <input type="text" bind:value={formAddresses[i].postal} />
                </label>
              </div>
              <div class="addr-row">
                <label class="addr-country">
                  {tl('contacts_country')}
                  <input type="text" bind:value={formAddresses[i].country} />
                </label>
                <label class="addr-type">
                  {tl('contacts_type')}
                  <select bind:value={formAddresses[i].type_label} class="type-select">
                    <option value="home">{tl('contacts_type_home')}</option>
                    <option value="work">{tl('contacts_type_work')}</option>
                    <option value="other">{tl('contacts_type_other')}</option>
                  </select>
                </label>
              </div>
            </li>
          {/each}
        </ul>
        <button type="button" class="outline small" onclick={() => { formAddresses = [...formAddresses, emptyAddress()]; }}>
          {tl('contacts_addAddress')}
        </button>
      </fieldset>

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
        <button type="button" class="secondary" onclick={closeForm}>{tl('contacts_cancel')}</button>
      </div>
    </form>
  </article>
</dialog>

<style>
  .contacts-page {
    width: 100%;
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

  /* Detail dialog */
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

  /* Form styles — grouped sections mirror .alarm-section in EventFormDialog */
  .contact-field-section {
    border: none;
    border-left: 3px solid var(--pico-primary-background);
    padding: 0 0 0 0.75rem;
    margin: 0 0 0.75rem;
  }

  .contact-field-section legend {
    font-size: 0.8125rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-weight: 600;
    color: var(--pico-muted-color);
    margin-bottom: 0.375rem;
  }

  .contact-field-list,
  .contact-addr-list {
    list-style: none;
    padding: 0;
    margin: 0 0 0.5rem;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
  }

  .contact-field-row {
    display: flex;
    align-items: center;
    gap: 0.5rem;
  }

  .contact-field-row input { flex: 1; }

  .contact-field-list input,
  .contact-field-list select {
    margin-bottom: 0;
  }

  .contact-addr-item { list-style: none; }

  .contact-addr-item + .contact-addr-item {
    padding-top: 0.75rem;
    border-top: 1px solid var(--pico-muted-border-color);
  }

  .contact-addr-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 0.5rem;
    font-size: 0.8125rem;
    font-weight: 600;
    color: var(--pico-muted-color);
  }

  .contact-addr-item label { margin-bottom: 0.4rem; }

  .contact-addr-item input,
  .contact-addr-item select {
    margin-bottom: 0;
  }

  .type-select {
    width: auto;
    min-width: 5.5rem;
    margin-bottom: 0;
    flex-shrink: 0;
  }

  .addr-row {
    display: flex;
    gap: 0.4rem;
    flex-wrap: wrap;
  }

  .addr-city { flex: 2; min-width: 8rem; }
  .addr-region { flex: 1; min-width: 5rem; }
  .addr-postal { flex: 1; min-width: 5rem; }
  .addr-country { flex: 2; min-width: 8rem; }
  .addr-type { flex: 1; min-width: 6rem; }
  .addr-type .type-select { width: 100%; }

  :global(button.small) { font-size: 0.875rem; padding: 0.25rem 0.75rem; }

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
