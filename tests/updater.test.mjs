import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const source = readFileSync(new URL('../src/updater.ts', import.meta.url), 'utf8');
const js = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
const { AppUpdater } = await import(`data:text/javascript;base64,${Buffer.from(js).toString('base64')}`);
function setup(overrides = {}) {
  const messages = [], calls = [];
  const update = {
    version: '9.0.0', body: 'Release notes',
    async download(onEvent) {
      calls.push('download');
      onEvent({ event: 'Started', data: { contentLength: 100 } });
      onEvent({ event: 'Progress', data: { chunkLength: 50 } });
      onEvent({ event: 'Finished' });
    },
    async install() { calls.push('install'); }, async close() {}
  };
  const deps = {
    async check() { calls.push('check'); return update; },
    async prepare() { calls.push('prepare'); }, async release() { calls.push('release'); },
    async restart() { calls.push('restart'); }, notify: (...args) => messages.push(args), ...overrides
  };
  return { updater: new AppUpdater(deps), deps, update, calls, messages };
}
test('startup is once only and no update or network error is silent; manual checks give feedback', async () => {
  for (const check of [async () => null, async () => { throw Error('offline'); }]) {
    const { updater, messages } = setup({ check });
    await updater.start(); await updater.start();
    assert.equal(messages.length, 0);
    await updater.check();
    assert.equal(messages.length, 1);
  }
});
test('update remains available, download is verified before ready, installation is explicit and guarded', async () => {
  const { updater, calls, messages, update } = setup();
  await updater.start();
  assert.equal(updater.snapshot().phase, 'available');
  assert.equal(messages.length, 1);
  update.download = async onEvent => {
    onEvent({ event: 'Started', data: {} });
    onEvent({ event: 'Progress', data: { chunkLength: 25 } });
    assert.equal(updater.snapshot().downloaded, 25);
    onEvent({ event: 'Finished' });
    assert.equal(updater.snapshot().phase, 'downloading', 'Finished fires before signature verification');
  };
  await updater.download();
  assert.equal(updater.snapshot().phase, 'ready');
  assert.equal(calls.includes('install'), false);
  await updater.install();
  assert.deepEqual(calls, ['check', 'prepare', 'install', 'restart']);
});
test('busy mail blocks install; failure releases gate; restart retry does not reinstall', async () => {
  const { updater, deps, calls } = setup();
  await updater.check(); await updater.download();
  deps.prepare = async () => { calls.push('busy'); throw Error('active mail'); };
  await updater.install();
  assert.equal(calls.includes('install'), false);
  assert.equal(updater.snapshot().phase, 'ready');
  deps.prepare = async () => { calls.push('prepare'); };
  deps.restart = async () => { calls.push('restart-failed'); throw Error('restart failed'); };
  await updater.install();
  assert.equal(updater.snapshot().phase, 'installed');
  assert.equal(calls.at(-1), 'release');
  deps.restart = async () => { calls.push('restart-ok'); };
  await updater.install();
  assert.equal(calls.filter(c => c === 'install').length, 1);
  assert.equal(calls.at(-1), 'restart-ok');
});
test('failed download or installation is retryable and concurrent clicks do not duplicate work', async () => {
  const { updater, update, calls } = setup();
  await updater.check();
  update.download = async () => { throw Error('bad signature'); };
  await updater.download();
  assert.equal(updater.snapshot().phase, 'available');
  let complete;
  update.download = () => new Promise(resolve => { complete = resolve; });
  const downloading = updater.download();
  await updater.download();
  complete(); await downloading;
  update.install = async () => { calls.push('failed-install'); throw Error('permission denied'); };
  await updater.install();
  assert.equal(updater.snapshot().phase, 'ready');
  assert.equal(calls.at(-1), 'release');
  update.install = async () => { calls.push('install'); };
  await updater.install();
  assert.equal(calls.at(-1), 'restart');
});
