// E-mail address lists without OpenPGP, so pages and helpers can parse them without loading openpgp.

/**
 * The commas of an address list that separate its entries (RFC 5322): not inside a quoted string (`"Doe, Jane"`,
 * with `\` escapes) and not inside an angle-address. Groups and comments are not understood.
 */
function separators(value: string): number[] {
  const at: number[] = [];
  let quoted = false;
  let angle = false;
  for (let i = 0; i < value.length; i++) {
    const c = value[i];
    if (quoted) {
      if (c === '\\') i++;
      else if (c === '"') quoted = false;
    } else if (angle) {
      if (c === '>') angle = false;
    } else if (c === '"') quoted = true;
    else if (c === '<') angle = true;
    else if (c === ',') at.push(i);
  }
  return at;
}

/** The entries of an address list (`"Doe, Jane" <j@x>, b@y` → `"Doe, Jane" <j@x>`, `b@y`), trimmed, empty ones left out. */
export function splitAddressList(value: string): string[] {
  const out: string[] = [];
  let start = 0;
  for (const i of [...separators(value), value.length]) {
    const entry = value.slice(start, i).trim();
    if (entry) out.push(entry);
    start = i + 1;
  }
  return out;
}

/** Where the entry being typed starts: after the last separating comma (0 without one). */
export function lastEntryStart(value: string): number {
  return (separators(value).at(-1) ?? -1) + 1;
}

/** The address of one entry: its last angle-address outside quotes (a name comes first and may look like one), else the entry. */
export function entryAddress(entry: string): string {
  let quoted = false;
  let open = -1;
  let found: string | null = null;
  for (let i = 0; i < entry.length; i++) {
    const c = entry[i];
    if (quoted) {
      if (c === '\\') i++;
      else if (c === '"') quoted = false;
    } else if (open >= 0) {
      if (c === '>') { found = entry.slice(open + 1, i); open = -1; }
    } else if (c === '"') quoted = true;
    else if (c === '<') open = i;
  }
  return (found ?? entry).trim();
}

/** The bare addresses of a comma-separated list (`Name <a@b>` → `a@b`). */
export function parseEnvelopeAddresses(value: string): string[] {
  return splitAddressList(value).map(entryAddress).filter(address => address.length > 0);
}

/** A plain `local@domain` address: nothing a list or a header would read as more than one address. */
export function isPlainAddress(value: string): boolean {
  return /^[^\s@<>",;:()[\]\\]+@[^\s@<>",;:()[\]\\]+$/.test(value);
}

/**
 * `Name <addr>` as the backend's `format_addr` writes it: the name without angle brackets, quoted (with `\` escapes)
 * when it holds a character that is special in a header (`, ; : " \ @ ( ) [ ]`); the bare address without a name.
 */
export function formatAddress(name: string, addr: string): string {
  const n = name.replace(/[<>]/g, '').trim();
  if (!n) return addr;
  const shown = /[,;:"\\@()[\]]/.test(n) ? `"${n.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"` : n;
  return `${shown} <${addr}>`;
}
