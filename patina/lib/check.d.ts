// Types for @tiaforge-ui/patina/check (lib/check.js). Written by hand; keep in step with lib/check.js.

/** One finding, in temper check's shape. line and col are 1-based. PT1xx: markup, PT2xx: CSS. */
export interface Diagnostic {
  file: string;
  line: number;
  col: number;
  severity: 'error' | 'warning';
  code: `PT${number}`;
  message: string;
}

/** patina's vocabulary (vocab.json, in temper's shape). */
export const VOCAB: { attributes: Record<string, unknown>; tokens: Record<string, { kind: 'input' | 'derived'; description: string }> };
/** Checks markup text: patina attributes only (temper's own rules are temper check's). */
export function checkMarkupText(text: string, file?: string): Diagnostic[];
/** Checks CSS text: --pt-* tokens. */
export function checkStylesText(text: string, file?: string): Diagnostic[];
/** Checks one file from disk. */
export function checkFile(file: string): Diagnostic[];
/** Whether any source under the paths imports '@tiaforge-ui/patina/patina.css'. */
export function importsPatina(paths: readonly string[]): boolean;
/** Checks every markup and CSS file under the paths; `css: false` skips PT110 (patina.css not imported). */
export function check(paths: readonly string[], options?: { css?: boolean }): Diagnostic[];
export function closest(word: string, candidates: Iterable<string>, max?: number): string | undefined;
