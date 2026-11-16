// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import assert from 'node:assert/strict';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import test from 'node:test';
// @ts-ignore Node 24 supplies this built-in module; browser typings may omit it.
import { readFileSync } from 'node:fs';
import { language, languages, matching, number, plural, resolve, rtl, subscribe, t, tn, use, type Catalog } from './i18n.ts';

const catalog = (code: string): Catalog => JSON.parse(readFileSync(new URL(`../locales/${code}.json`, import.meta.url), 'utf8'));
const english = catalog('en');
const forms = ['zero', 'one', 'two', 'few', 'many', 'other'];
const pluralBase = (key: string) => { const at = key.lastIndexOf('.'); return forms.includes(key.slice(at + 1)) ? key.slice(0, at) : null; };
const placeholders = (text: string) => [...text.matchAll(/\{([A-Za-z0-9_]+)\}/g)].map(match => match[1]).sort();

test('every catalog has every key, with the same placeholders', () => {
  const plurals = new Set(Object.keys(english).map(pluralBase).filter((base): base is string => !!base));
  for (const { code } of languages) {
    const own = catalog(code);
    for (const [key, text] of Object.entries(english)) {
      if (pluralBase(key)) continue;
      assert.ok(own[key]?.trim(), `${code}.json has no ${key}`);
      assert.deepEqual(placeholders(own[key]), placeholders(text), `${code}.json ${key}`);
    }
    // Every form the language uses for whole numbers is there.
    const used = new Set(Array.from({ length: 201 }, (_, count) => new Intl.PluralRules(code).select(count)));
    for (const base of plurals) {
      const wanted = placeholders(english[`${base}.other`]).filter(name => name !== 'count');
      for (const form of used) assert.ok(own[`${base}.${form}`], `${code}.json has no ${base}.${form}`);
      for (const form of forms) {
        const text = own[`${base}.${form}`];
        if (text !== undefined) assert.deepEqual(placeholders(text).filter(name => name !== 'count'), wanted, `${code}.json ${base}.${form}`);
      }
    }
  }
});

test('a key falls back to English, then to itself; placeholders are filled', () => {
  use('de', { 'a.hello': 'Hallo {name}' }, { 'a.hello': 'Hi {name}', 'a.only': 'Only in English' });
  assert.equal(language(), 'de');
  assert.equal(t('a.hello', { name: 'Ana' }), 'Hallo Ana');
  assert.equal(t('a.only'), 'Only in English');
  assert.equal(t('a.missing'), 'a.missing');
  assert.equal(t('a.hello'), 'Hallo {name}', 'a placeholder without a value stays');
});

test('a count picks its plural form and is written the language’s way', () => {
  const en = { 'a.files.one': '{count} file', 'a.files.other': '{count} files' };
  use('en', en, en);
  assert.equal(tn('a.files', 1), '1 file');
  assert.equal(tn('a.files', 1200), '1,200 files');
  use('ru', { 'a.files.one': '{count} файл', 'a.files.few': '{count} файла', 'a.files.many': '{count} файлов', 'a.files.other': '{count} файла' }, en);
  assert.equal(tn('a.files', 21), '21 файл');
  assert.equal(tn('a.files', 3), '3 файла');
  assert.equal(tn('a.files', 11), '11 файлов');
  use('ar', { 'a.files.other': '{count} ملف' }, en);
  assert.equal(tn('a.files', 100), `${number(100)} ملف`, 'a missing form uses other');
  use('ja', {}, en);
  assert.equal(tn('a.files', 2), '2 files', 'a missing key uses English');
  assert.equal(plural('ar', 0), 'zero');
  assert.equal(plural('fr', 0), 'one');
});

test('the language comes from Settings, then the Mac, then English', () => {
  assert.equal(resolve('de', ['fr-FR']), 'de');
  assert.equal(resolve('', ['nb-NO', 'de-AT']), 'de');
  assert.equal(resolve('', ['sv-SE']), 'en');
  assert.equal(resolve('klingon', ['ja-JP']), 'ja');
  assert.equal(matching('zh-Hant-TW'), 'zh-Hant');
  assert.equal(matching('zh-HK'), 'zh-Hant');
  assert.equal(matching('zh-CN'), 'zh-Hans');
  assert.equal(matching('pt-PT'), 'pt-BR');
  assert.equal(matching('es-419'), 'es');
  assert.equal(matching('in-ID'), 'id');
  assert.equal(matching('sv'), undefined);
  assert.ok(rtl('ar') && !rtl('en'));
  assert.equal(languages.length, 16);
});

test('a change of language is announced once', () => {
  const en = { 'a.x': 'x' };
  use('en', en, en);
  let heard = 0;
  const stop = subscribe(() => { heard += 1; });
  use('en', en, en);
  assert.equal(heard, 0);
  use('fr', {}, en);
  assert.equal(heard, 1);
  stop();
  use('en', en, en);
  assert.equal(heard, 1);
});
