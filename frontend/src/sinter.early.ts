// Runs in every page's early chunk (sinter), before the first paint and before any binding: the language the
// rail's labels and the other early text render in. Keep it light: it is render-blocking.
import { initLanguage } from './lib/language.ts';

initLanguage();
