<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import { makeTl } from '../lib/i18n.ts';
  import type { EventJson, ContactJson } from '../types/calendar.ts';
  import { fetchEventICS, fetchContactVCF, downloadFile, composeEmailWithAttachment, safeFilename } from '../lib/share.ts';

  export let event: EventJson | null = null;
  export let contact: ContactJson | null = null;
  export let locale: string = 'en';

  $: tl = makeTl(locale);

  let useMobileShare = false;
  let popoverOpen = false;
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
    popoverOpen = false;
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
    popoverOpen = false;
    loading = true;
    try {
      const content = await getContent();
      composeEmailWithAttachment(getTitle(), getFilename(), getMimeType(), content);
    } catch (err) {
      console.error('Email share failed:', err);
      loading = false;
    }
  }

  function handleWindowClick(e: MouseEvent) {
    if (popoverOpen && !(e.target as Element).closest('.share-wrap')) {
      popoverOpen = false;
    }
  }

  onMount(() => {
    useMobileShare = window.matchMedia('(pointer: coarse)').matches && typeof navigator.share === 'function';
    window.addEventListener('click', handleWindowClick);
  });

  onDestroy(() => {
    window.removeEventListener('click', handleWindowClick);
  });
</script>

<div class="share-wrap">
  <button
    class="share-btn"
    title={tl('share_share')}
    disabled={loading}
    onclick={() => useMobileShare ? handleShare() : (popoverOpen = !popoverOpen)}
  >
    {#if loading}
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" class="spin">
        <path d="M21 12a9 9 0 1 1-6.219-8.56"/>
      </svg>
    {:else}
      <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <circle cx="18" cy="5" r="3"/>
        <circle cx="6" cy="12" r="3"/>
        <circle cx="18" cy="19" r="3"/>
        <line x1="8.59" y1="13.51" x2="15.42" y2="17.49"/>
        <line x1="15.41" y1="6.51" x2="8.59" y2="10.49"/>
      </svg>
    {/if}
  </button>

  {#if popoverOpen}
    <div class="share-popover">
      <button onclick={handleDownload}>
        {event ? tl('share_downloadIcs') : tl('share_downloadVCard')}
      </button>
      <button onclick={handleEmail}>
        {tl('share_email')}
      </button>
    </div>
  {/if}
</div>

<style>
  .share-wrap { position: relative; }

  .share-btn {
    background: none; border: none; cursor: pointer;
    color: var(--pico-muted-color);
    padding: 0.25rem; line-height: 1; transition: color 0.15s;
    display: flex; align-items: center;
  }
  .share-btn:hover:not(:disabled) { color: var(--pico-primary-background); }
  .share-btn:disabled { cursor: default; opacity: 0.5; }

  .spin {
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    from { transform: rotate(0deg); }
    to { transform: rotate(360deg); }
  }

  .share-popover {
    position: absolute;
    top: calc(100% + 0.25rem);
    right: 0;
    z-index: 10;
    display: flex;
    flex-direction: column;
    gap: 0.125rem;
    padding: 0.25rem;
    background: var(--pico-background-color);
    border: 1px solid var(--pico-muted-border-color);
    border-radius: var(--pico-border-radius);
    box-shadow: 0 0.25rem 1rem rgba(0,0,0,0.2);
    white-space: nowrap;
  }
  .share-popover button {
    background: none; border: none; text-align: left; font-size: 0.875rem;
    cursor: pointer; padding: 0.5rem 0.75rem; border-radius: var(--pico-border-radius);
    color: var(--pico-color); white-space: nowrap;
  }
  .share-popover button:hover { background: var(--pico-primary-background); color: var(--pico-primary-inverse); }
</style>
