<script lang="ts">
  import { tick } from 'svelte';
  import { makeTl } from '../../lib/i18n.ts';
  import type { AddressbookJson, ContactJson, TypedEntry, AddressEntry } from '../../types/calendar.ts';

  export let tl: ReturnType<typeof makeTl>;
  export let addressbooks: AddressbookJson[] = [];
  export let onsaved: (contact: ContactJson) => void = () => {};

  const emptyEmail = (): TypedEntry => ({ value: '', type_label: 'other' });
  const emptyPhone = (): TypedEntry => ({ value: '', type_label: 'other' });
  const emptyAddress = (): AddressEntry => ({ street: '', city: '', region: '', postal: '', country: '', type_label: 'home' });

  let dialog: HTMLDialogElement;
  let formNameInput: HTMLInputElement;
  let formUid: string | null = null;
  let formName = '';
  let formEmails: TypedEntry[] = [emptyEmail()];
  let formPhones: TypedEntry[] = [emptyPhone()];
  let formAddresses: AddressEntry[] = [];
  let formOrg = '';
  let formAccountId = '';
  let formAddressbookHref = '';
  let formSaving = false;
  let formError = '';

  export async function openNew() {
    formUid = null;
    formName = '';
    formEmails = [emptyEmail()];
    formPhones = [emptyPhone()];
    formAddresses = [];
    formOrg = '';
    formAccountId = addressbooks[0]?.account_id ?? '';
    formAddressbookHref = addressbooks[0]?.href ?? '';
    formError = '';
    await tick();
    dialog?.showModal();
    dialog?.querySelector('article')?.scrollTo(0, 0);
    formNameInput?.select();
  }

  export async function openEdit(c: ContactJson) {
    formUid = c.uid;
    formName = c.fn_name;
    formEmails = c.emails.length > 0 ? c.emails.map(e => ({ ...e })) : [emptyEmail()];
    formPhones = c.phones.length > 0 ? c.phones.map(p => ({ ...p })) : [emptyPhone()];
    formAddresses = c.addresses.map(a => ({ ...a }));
    formOrg = c.org;
    formAccountId = c.account_id;
    formAddressbookHref = c.addressbook_href;
    formError = '';
    await tick();
    dialog?.showModal();
    dialog?.querySelector('article')?.scrollTo(0, 0);
    formNameInput?.select();
  }

  export function isOpen(): boolean {
    return dialog?.open ?? false;
  }

  function closeForm() {
    dialog?.close();
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
          account_id: formAccountId,
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
        onsaved(saved);
      }
    } catch {
      formError = tl('contacts_error');
    } finally {
      formSaving = false;
    }
  }
</script>

<!-- svelte-ignore a11y-click-events-have-key-events a11y-no-noninteractive-element-interactions -->
<dialog
  bind:this={dialog}
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
          <select
            value={`${formAccountId}\0${formAddressbookHref}`}
            onchange={(e) => {
              const [accountId, href] = (e.target as HTMLSelectElement).value.split('\0');
              formAccountId = accountId;
              formAddressbookHref = href;
            }}
          >
            {#each addressbooks as ab (ab.account_id + ab.href)}
              <option value={`${ab.account_id}\0${ab.href}`}>{ab.name}</option>
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
  .header-actions {
    display: flex;
    align-items: center;
    gap: 0.25rem;
  }

  .contact-field-section {
    border: none;
    border-left: 3px solid var(--pico-primary-background);
    padding: 0 0 0 0.75rem;
    margin: 0 0 0.75rem;
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
</style>
