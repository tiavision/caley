// E-mail address lists without OpenPGP, so pages and helpers can parse them without loading openpgp.

/** The bare addresses of a comma-separated list (`Name <a@b>` → `a@b`). */
export function parseEnvelopeAddresses(value: string): string[] {
  return value
    .split(',')
    .map(address => {
      const match = /<([^<>]+)>/.exec(address);
      return (match?.[1] ?? address).trim();
    })
    .filter(address => address.length > 0);
}
