// Cursor's local SDK runtime with no model tools or ambient settings.
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const [sdk, runFolder, model] = process.argv.slice(1);
const send = (type, fields = {}) => process.stdout.write(JSON.stringify({ type, ...fields }) + '\n');
function fail(message) {
  send('error', { message });
  process.exitCode = 1;
}

try {
  const { Agent, AuthenticationError, Cursor, JsonlLocalAgentStore } = await import(pathToFileURL(sdk).href);
  if ((await Cursor.auth.status()).status !== 'logged-in') {
    fail('Cursor login required.');
  } else {
    await answer(Agent, AuthenticationError, JsonlLocalAgentStore);
  }
} catch {
  fail('Could not start the supported Cursor SDK.');
}

async function answer(Agent, AuthenticationError, JsonlLocalAgentStore) {
  const workspace = join(runFolder, 'workspace');
  await mkdir(workspace, { mode: 0o700 });
  let agent;
  try {
    agent = await Agent.create({
      model: { id: model || 'auto' },
      tools: [],
      disallowedTools: ['mcp'],
      local: {
        cwd: workspace,
        settingSources: [],
        sandboxOptions: { enabled: true },
        store: new JsonlLocalAgentStore(join(runFolder, 'store')),
      },
    });
    // Read the piped prompt after configuring the runtime.
    const chunks = [];
    for await (const chunk of process.stdin) chunks.push(chunk);
    const run = await agent.send(Buffer.concat(chunks).toString('utf8'));
    for await (const event of run.stream()) {
      if (event.type === 'tool_call' ||
          (event.type === 'system' && event.tools?.length) ||
          (event.type === 'assistant' && event.message.content.some(block => block.type === 'tool_use'))) {
        send('tool');
        await run.cancel();
        return;
      }
    }
    const result = await run.wait();
    if (result.status === 'finished' && typeof result.result === 'string') {
      send('result', { text: result.result });
    } else {
      const needsLogin = /unauthenticated|unauthorized|invalid.api.key|expired.api.key/i.test(result.error?.code ?? '');
      fail(needsLogin ? 'Cursor login required.' : 'The Cursor SDK turn failed.');
    }
  } catch (error) {
    fail(error instanceof AuthenticationError ? 'Cursor login required.' : 'The Cursor SDK could not start or finish the turn.');
  } finally {
    if (agent) await agent[Symbol.asyncDispose]();
  }
}
