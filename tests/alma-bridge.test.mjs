import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';

const bridge = readFileSync(new URL('../crates/core/src/alma_bridge.js', import.meta.url), 'utf8');
async function runBridge({ busy = false, failure = false } = {}) {
  const sent = [], output = [], errors = [];
  const process = {
    stdin: (async function* () { yield JSON.stringify({ threadId: 'original-thread', prompt: 'Update the project' }); })(),
    stdout: { write: text => output.push(text) }, stderr: { write: text => errors.push(text) }, exitCode: 0,
  };
  class Socket {
    listeners = {};
    constructor() { queueMicrotask(() => this.listeners.open()); }
    addEventListener(type, handler) { this.listeners[type] = handler; }
    close() {}
    send(raw) {
      sent.push(JSON.parse(raw));
      queueMicrotask(() => {
        const emit = (type, data) => this.listeners.message({ data: JSON.stringify({ type, data }) });
        if (failure) return emit('generation_error', { threadId: 'original-thread', error: 'Native approval required' });
        emit('message_delta', { threadId: 'original-thread', deltas: [{ type: 'text_append', partType: 'text', text: 'Done' }] });
        emit('thread_generating', { id: 'original-thread', isGenerating: false });
      });
    }
  }
  await runInNewContext(bridge, {
    process, WebSocket: Socket, AbortSignal, setTimeout, clearTimeout,
    fetch: async () => ({ ok: true, json: async () => ({ id: 'original-thread', model: 'original-model', isGenerating: busy }) }),
  });
  return { sent, output, errors, exitCode: process.exitCode };
}

test('Alma continues the original thread/model without tool or permission overrides', async () => {
  const result = await runBridge();
  assert.equal(result.exitCode, 0);
  assert.equal(result.output.join(''), 'Done');
  assert.deepEqual(result.sent, [{ type: 'generate_response', data: {
    threadId: 'original-thread', model: 'original-model', source: 'ema',
    userMessage: { role: 'user', parts: [{ type: 'text', text: 'Update the project' }] },
  } }]);
});
test('Alma reports native approval failures without approving or retrying', async () => {
  const result = await runBridge({ failure: true });
  assert.equal(result.exitCode, 1);
  assert.equal(result.errors.join(''), 'Native approval required');
  assert.equal(result.sent.length, 1);
});
test('Alma still refuses a busy original session', async () => {
  const result = await runBridge({ busy: true });
  assert.equal(result.exitCode, 1);
  assert.match(result.errors.join(''), /busy/);
  assert.equal(result.sent.length, 0);
});
