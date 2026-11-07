<script lang="ts">
  import { tick } from 'svelte';
  import { makeTl } from '../../lib/i18n.ts';
  import { apiFetch } from '../../lib/api.ts';
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
    if (!dialog?.open) dialog?.showModal();
    dialog?.scrollTo(0, 0);
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
    if (!dialog?.open) dialog?.showModal();
    dialog?.scrollTo(0, 0);
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
      const res = await apiFetch('/api/contact', {
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

<dialog
  bind:this={dialog}
  id="contact-form"
  closedby="any"
  style="--tm-dialog-width: 36rem"
>
  <header>
    <h2>{formUid ? tl('contacts_edit') : tl('contacts_new')}</h2>
    <button type="button" commandfor="contact-form" command="close" aria-label={tl('weekly_aria_close')}></button>
  </header>

  <form data-layout="stack" data-gap="md" onsubmit={saveContact}>
    {#if formError}
      <div data-callout data-tone="danger" role="alert">{formError}</div>
    {/if}

    <label>
      {tl('contacts_name')}
      <input type="text" bind:value={formName} bind:this={formNameInput} required />
    </label>

    <fieldset data-layout="stack" data-gap="sm">
      <legend>{tl('contacts_email')}</legend>
      {#each formEmails as entry, i}
        <div data-layout="row" data-gap="sm" data-wrap="none">
          <input type="email" data-grow aria-label={tl('contacts_email')} bind:value={formEmails[i].value} />
          <select aria-label={tl('contacts_type')} bind:value={formEmails[i].type_label}>
            <option value="home">{tl('contacts_type_home')}</option>
            <option value="work">{tl('contacts_type_work')}</option>
            <option value="other">{tl('contacts_type_other')}</option>
          </select>
          <button type="button" data-icon="x" data-variant="ghost" aria-label={tl('contacts_remove')} onclick={() => { formEmails = formEmails.filter((_, j) => j !== i); }}></button>
        </div>
      {/each}
      <button type="button" data-variant="outline" data-size="sm" data-icon="plus" data-self="start" onclick={() => { formEmails = [...formEmails, emptyEmail()]; }}>
        {tl('contacts_addEmail')}
      </button>
    </fieldset>

    <fieldset data-layout="stack" data-gap="sm">
      <legend>{tl('contacts_phone')}</legend>
      {#each formPhones as entry, i}
        <div data-layout="row" data-gap="sm" data-wrap="none">
          <input type="tel" data-grow aria-label={tl('contacts_phone')} bind:value={formPhones[i].value} />
          <select aria-label={tl('contacts_type')} bind:value={formPhones[i].type_label}>
            <option value="home">{tl('contacts_type_home')}</option>
            <option value="work">{tl('contacts_type_work')}</option>
            <option value="mobile">{tl('contacts_type_mobile')}</option>
            <option value="fax">{tl('contacts_type_fax')}</option>
            <option value="pager">{tl('contacts_type_pager')}</option>
            <option value="other">{tl('contacts_type_other')}</option>
          </select>
          <button type="button" data-icon="x" data-variant="ghost" aria-label={tl('contacts_remove')} onclick={() => { formPhones = formPhones.filter((_, j) => j !== i); }}></button>
        </div>
      {/each}
      <button type="button" data-variant="outline" data-size="sm" data-icon="plus" data-self="start" onclick={() => { formPhones = [...formPhones, emptyPhone()]; }}>
        {tl('contacts_addPhone')}
      </button>
    </fieldset>

    <fieldset data-layout="stack" data-gap="sm">
      <legend>{tl('contacts_address')}</legend>
      {#each formAddresses as addr, i}
        <div data-layout="stack" data-gap="sm">
          <div data-layout="row" data-gap="sm" data-justify="between" data-wrap="none">
            <span data-caption>{tl('contacts_address')}{formAddresses.length > 1 ? ` ${i + 1}` : ''}</span>
            <button type="button" data-icon="x" data-variant="ghost" data-size="sm" aria-label={tl('contacts_remove')} onclick={() => { formAddresses = formAddresses.filter((_, j) => j !== i); }}></button>
          </div>
          <label>
            {tl('contacts_street')}
            <input type="text" bind:value={formAddresses[i].street} />
          </label>
          <div data-layout="grid" data-gap="sm" style="--tm-min: 8rem">
            <label>
              {tl('contacts_city')}
              <input type="text" bind:value={formAddresses[i].city} />
            </label>
            <label>
              {tl('contacts_region')}
              <input type="text" bind:value={formAddresses[i].region} />
            </label>
            <label>
              {tl('contacts_postal')}
              <input type="text" bind:value={formAddresses[i].postal} />
            </label>
          </div>
          <div data-layout="grid" data-gap="sm" style="--tm-min: 8rem">
            <label>
              {tl('contacts_country')}
              <input type="text" bind:value={formAddresses[i].country} />
            </label>
            <label>
              {tl('contacts_type')}
              <select bind:value={formAddresses[i].type_label}>
                <option value="home">{tl('contacts_type_home')}</option>
                <option value="work">{tl('contacts_type_work')}</option>
                <option value="other">{tl('contacts_type_other')}</option>
              </select>
            </label>
          </div>
        </div>
        {#if i < formAddresses.length - 1}<hr />{/if}
      {/each}
      <button type="button" data-variant="outline" data-size="sm" data-icon="plus" data-self="start" onclick={() => { formAddresses = [...formAddresses, emptyAddress()]; }}>
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

    <footer>
      <button type="button" data-variant="ghost" commandfor="contact-form" command="close">{tl('contacts_cancel')}</button>
      <button type="submit" aria-busy={formSaving} disabled={formSaving}>{tl('contacts_save')}</button>
    </footer>
  </form>
</dialog>
