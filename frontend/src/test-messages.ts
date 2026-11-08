// Tests: the message files as sinter's testing module takes them (`render(page, { messages })`), English first.
import en from '../messages/en.json';
import de from '../messages/de.json';

const plain = (o: Record<string, unknown>) => Object.fromEntries(Object.entries(o).filter(([k]) => k !== '$schema')) as Record<string, string>;

export const messages = { en: plain(en), de: plain(de) };

// Unit tests of helpers that call `m` read English, as a page with no stored choice would (render() sets its own).
(globalThis as Record<symbol, unknown>)[Symbol.for('sinter.i18n')] ??= { locale: 'en', base: 'en', locales: ['en', 'de'], storage: 'locale', files: {}, dicts: messages };
