// The bundled Alma runtime provides fetch and WebSocket. No prompt is passed in argv.
(async () => {
  let input = '';
  for await (const chunk of process.stdin) input += chunk;
  const { threadId, prompt } = JSON.parse(input);
  const response = await fetch(`http://127.0.0.1:23001/api/threads/${threadId}`, {
    signal: AbortSignal.timeout(10000),
  });
  if (!response.ok) throw new Error(`Original Alma session unavailable (${response.status})`);
  const thread = await response.json();
  if (thread.id !== threadId || !thread.model) throw new Error('Original Alma session or model unavailable');
  if (thread.isGenerating) throw new Error('Original Alma session is busy; reply again after it finishes');
  if (typeof WebSocket === 'undefined') throw new Error('This runtime does not support WebSocket');
  const text = await new Promise((resolve, reject) => {
    const ws = new WebSocket('ws://127.0.0.1:23001/ws/threads');
    let started = false, answer = '', settled = false, finishTimer;
    const timer = setTimeout(() => finish(new Error('Alma timed out; the session may still be running')), 900000);
    function finish(error) {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      clearTimeout(finishTimer);
      ws.close();
      if (error) reject(error); else resolve(answer);
    }
    ws.addEventListener('open', () => ws.send(JSON.stringify({
      type: 'generate_response', data: { threadId, model: thread.model, noTools: true, source: 'ema',
        userMessage: { role: 'user', parts: [{ type: 'text', text: prompt }] } },
    })));
    ws.addEventListener('error', () => finish(new Error('Could not connect to Alma')));
    ws.addEventListener('close', () => finish(new Error('Alma disconnected; the session may still be running')));
    ws.addEventListener('message', event => {
      let message;
      try { message = JSON.parse(String(event.data)); } catch { return; }
      const data = message.data || {};
      if (message.type === 'error') return finish(new Error(data.error || 'Alma rejected the request'));
      if (data.threadId === threadId) {
        if (message.type === 'generation_error') return finish(new Error(data.error || 'Alma generation failed'));
        if (message.type === 'message_delta') {
          started = true;
          for (const delta of data.deltas || []) {
            if (delta.type === 'text_append' && delta.partType === 'text') answer += delta.text || '';
          }
          if (answer.length > 100000) return finish(new Error('Alma response exceeded the size limit'));
        }
      }
      if (message.type === 'thread_generating' && data.id === threadId) {
        if (data.isGenerating === true) started = true;
        if (started && data.isGenerating === false) {
          // Alma can broadcast generation_error immediately after the idle event.
          finishTimer = setTimeout(() => finish(answer.trim() ? null : new Error('Alma finished without a text response')), 300);
        }
      }
    });
  });
  process.stdout.write(text);
})().catch(error => { process.stderr.write(String(error.message || error)); process.exitCode = 1; });
