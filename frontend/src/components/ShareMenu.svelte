<script lang="ts" module>
  let nextId = 0;
</script>

<script lang="ts">
  import { onMount } from 'svelte';
  import { makeTl } from '../lib/i18n.ts';
  import type { EventJson, ContactJson } from '../types/calendar.ts';
  import { fetchEventICS, fetchContactVCF, downloadFile, composeEmailWithAttachment, safeFilename } from '../lib/share.ts';

  export let event: EventJson | null = null;
  export let contact: ContactJson | null = null;
  export let locale: string = 'en';

  $: tl = makeTl(locale);

  let useMobileShare = false;
  const menuId = `share-menu-${++nextId}`;
  let menu: HTMLElement;
  let loading = false;

  function getTitle(): string {
    return event?.summary || contact?.fn_name || '';
  }

  function getFilename(): string {
    if (event) return safeFilename(event.summary || 'event', '.ics');
    if (contact) return safeFilename(contact.fn_name || 'contact', '.vcf');
    return 'file';
  }

  function getMimeType(): string {
    return event ? 'text/calendar' : 'text/vcard';
  }

  async function getContent(): Promise<string> {
    if (event) return fetchEventICS(event);
    if (contact) return fetchContactVCF(contact);
    throw new Error('no item');
  }

  async function handleShare() {
    loading = true;
    try {
      const content = await getContent();
      const file = new File([content], getFilename(), { type: getMimeType() });
      if (navigator.canShare?.({ files: [file] })) {
        await navigator.share({ title: getTitle(), files: [file] });
      } else {
        await navigator.share({ title: getTitle() });
      }
    } catch (err: unknown) {
      if (err instanceof Error && err.name !== 'AbortError') {
        console.error('Share failed:', err);
      }
    } finally {
      loading = false;
    }
  }

  async function handleDownload() {
    menu?.hidePopover();
    loading = true;
    try {
      const content = await getContent();
      downloadFile(getFilename(), content, getMimeType());
    } catch (err) {
      console.error('Download failed:', err);
    } finally {
      loading = false;
    }
  }

  async function handleEmail() {
    menu?.hidePopover();
    loading = true;
    try {
      const content = await getContent();
      composeEmailWithAttachment(getTitle(), getFilename(), getMimeType(), content);
    } catch (err) {
      console.error('Email share failed:', err);
      loading = false;
    }
  }

  onMount(() => {
    useMobileShare = window.matchMedia('(pointer: coarse)').matches && typeof navigator.share === 'function';
  });
</script>

<button
  type="button"
  data-icon="share-2"
  data-variant="ghost"
  aria-label={tl('share_share')}
  aria-busy={loading ? 'true' : null}
  disabled={loading}
  popovertarget={useMobileShare ? null : menuId}
  onclick={() => { if (useMobileShare) handleShare(); }}
></button>

<div popover id={menuId} data-menu bind:this={menu}>
  <button type="button" data-icon="download" onclick={handleDownload}>
    {event ? tl('share_downloadIcs') : tl('share_downloadVCard')}
  </button>
  <button type="button" data-icon="mail" onclick={handleEmail}>
    {tl('share_email')}
  </button>
</div>
