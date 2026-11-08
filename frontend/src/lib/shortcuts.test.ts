import { describe, expect, test } from 'vitest';
import { shortcutMatches, typingTarget } from './shortcuts.ts';

const key = (k: string, mods: Partial<KeyboardEvent> = {}) =>
  ({ key: k, ctrlKey: false, altKey: false, metaKey: false, shiftKey: false, ...mods }) as KeyboardEvent;

describe('shortcutMatches', () => {
  test('a letter: that key without modifiers, Shift included', () => {
    expect(shortcutMatches('c', key('c'))).toBe(true);
    expect(shortcutMatches('c', key('C', { shiftKey: true }))).toBe(false);
    expect(shortcutMatches('c', key('c', { ctrlKey: true }))).toBe(false);
    expect(shortcutMatches('c', key('c', { metaKey: true }))).toBe(false);
    expect(shortcutMatches('c', key('c', { altKey: true }))).toBe(false);
    expect(shortcutMatches('c', key('d'))).toBe(false);
  });
  test('shift-<letter> needs Shift, in either case', () => {
    expect(shortcutMatches('shift-i', key('I', { shiftKey: true }))).toBe(true);
    expect(shortcutMatches('shift-i', key('i', { shiftKey: true }))).toBe(true);
    expect(shortcutMatches('shift-i', key('i'))).toBe(false);
    expect(shortcutMatches('shift-i', key('I', { shiftKey: true, ctrlKey: true }))).toBe(false);
  });
  test('other characters match however they are typed', () => {
    expect(shortcutMatches('#', key('#', { shiftKey: true }))).toBe(true);
    expect(shortcutMatches('#', key('#'))).toBe(true);
    expect(shortcutMatches('#', key('#', { altKey: true }))).toBe(false);
  });
  test('named keys and Ctrl', () => {
    expect(shortcutMatches('escape', key('Escape'))).toBe(true);
    expect(shortcutMatches('escape', key('Escape', { shiftKey: true }))).toBe(false);
    expect(shortcutMatches('ctrl-enter', key('Enter', { ctrlKey: true }))).toBe(true);
    expect(shortcutMatches('ctrl-enter', key('Enter'))).toBe(false);
  });
});

test('typingTarget: fields and contenteditable take the keys, buttons do not', () => {
  document.body.innerHTML = '<input><textarea></textarea><select></select><div contenteditable="true"><b></b></div><button></button>';
  const [input, area, select, div, b, button] = ['input', 'textarea', 'select', 'div', 'b', 'button'].map((s) => document.querySelector(s));
  expect([input, area, select, div, b].map(typingTarget)).toEqual([true, true, true, true, true]);
  expect(typingTarget(button)).toBe(false);
  expect(typingTarget(null)).toBe(false);
});
