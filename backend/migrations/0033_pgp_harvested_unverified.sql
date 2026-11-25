-- Keys found in mail in the background (Autocrypt headers, attachments) were stored as trusted, without the user
-- seeing them. They now wait to be accepted: `unverified` counts for neither signatures nor encryption. Nothing is
-- deleted; the user accepts or removes them in Settings (or in compose, for a recipient).
UPDATE pgp_public_keys
SET trust = 'unverified'
WHERE source IN ('harvested_autocrypt', 'harvested_attachment') AND is_own = 0;
