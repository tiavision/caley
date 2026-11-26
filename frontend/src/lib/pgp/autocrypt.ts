import * as openpgp from 'openpgp';

function headerValues(raw: string, name: string): string[] {
  const headerBlock = raw.split(/\r?\n\r?\n/, 1)[0] ?? '';
  const unfolded = headerBlock.replace(/\r?\n[ \t]+/g, ' ');
  return unfolded
    .split(/\r?\n/)
    .filter(line => line.slice(0, name.length + 1).toLowerCase() === `${name.toLowerCase()}:`)
    .map(line => line.slice(name.length + 1).trim());
}

function userEmails(key: openpgp.PublicKey): string[] {
  return key.users
    .map(user => user.userID?.email?.trim().toLowerCase())
    .filter((email): email is string => !!email);
}

/** The keys the message's Autocrypt headers give for `sender` (the sender's address, Message.from_address). */
export async function autocryptKeys(rawBytes: Uint8Array, sender: string | null): Promise<openpgp.PublicKey[]> {
  sender = sender?.trim().toLowerCase() ?? '';
  if (!sender) return [];
  const raw = new TextDecoder().decode(rawBytes);
  const keys: openpgp.PublicKey[] = [];

  for (const value of headerValues(raw, 'Autocrypt')) {
    const params = new Map<string, string>();
    for (const part of value.split(';')) {
      const index = part.indexOf('=');
      if (index > 0) params.set(part.slice(0, index).trim().toLowerCase(), part.slice(index + 1).trim());
    }
    const addr = params.get('addr')?.toLowerCase();
    const keydata = params.get('keydata')?.replace(/\s+/g, '');
    if (!addr || addr !== sender || !keydata) continue;
    try {
      const binary = Uint8Array.from(atob(keydata), char => char.charCodeAt(0));
      const key = await openpgp.readKey({ binaryKey: binary });
      if (userEmails(key).includes(addr)) keys.push(key);
    } catch {
      // Malformed advertisements are ignored.
    }
  }
  return keys;
}
