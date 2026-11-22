// The content module (sinter's fixed name): the site's languages, which `localePrefix` builds every page in, and the
// messages (`m.key()`), both from the inlang project (project.inlang/settings.json: `en` base, `de`; messages/{locale}.json).
import { languages } from '@tiaforge-ui/sinter/content';
import { inlang, inlangLocales } from '@tiaforge-ui/sinter/messages/content';

export const locales = languages(({ root }) => inlangLocales('project.inlang', root));
export const messages = inlang();
