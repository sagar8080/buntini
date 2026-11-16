import { useEffect, useState } from 'react';
import { Check, LoaderCircle, Share } from 'lucide-react';
import Toggle from '../Toggle';
import { command, errorMessage } from '../bridge';
import type { CodingAgent } from '../types';
import { callAgents } from './api';
import type { RecapWeek } from './model';

/** What each agent can do from the cat, by what its hooks allow. */
const can: Record<string, string> = {
  claude: 'Approve, deny and answer from the cat · Know when it’s done',
  codex: 'Approve and deny from the cat · Know when it’s done',
};
const startable = ['claude', 'codex'];

/** Settings → Apps → Coding agents: hooks on and off, plan usage, a new session, the week. */
export default function AgentsSection({ onError }: { onError: (error: string) => void }) {
  const [agents, setAgents] = useState<CodingAgent[]>([]);
  const [busy, setBusy] = useState('');
  const [projects, setProjects] = useState<string[]>([]);
  const [week, setWeek] = useState<RecapWeek | null>(null);
  const [starting, setStarting] = useState({ agent: 'claude', project: '', prompt: '' });
  useEffect(() => {
    command<CodingAgent[]>('coding_agents').then(setAgents).catch(() => {});
    callAgents<string[]>({ op: 'projects' }).then(found => { setProjects(found); setStarting(previous => ({ ...previous, project: previous.project || found[0] || '' })); }).catch(() => {});
    callAgents<RecapWeek[]>({ op: 'recap' }).then(weeks => setWeek(weeks[0] ?? null)).catch(() => {});
  }, []);
  const replace = (next: CodingAgent) => setAgents(previous => previous.map(agent => agent.id === next.id ? next : agent));
  async function act(key: string, run: () => Promise<void>) {
    setBusy(key);
    try { await run(); } catch (error) { onError(errorMessage(error)); } finally { setBusy(''); }
  }
  const connect = (id: string, on: boolean) => act(id, async () => replace(await command<CodingAgent>('connect_coding_agent', { id, on })));
  const usage = (agent: CodingAgent, on: boolean) => act(`${agent.id}-usage`, async () => {
    await callAgents({ op: 'planUsage', on });
    replace({ ...agent, usage: on });
  });
  const chooseFolder = () => act('folder', async () => {
    const path = await command<string | null>('pick_folder');
    if (path) { setProjects(previous => [path, ...previous.filter(item => item !== path)]); setStarting(previous => ({ ...previous, project: path })); }
  });
  const start = () => act('start', async () => {
    await callAgents({ op: 'start', agent: starting.agent, project: starting.project, prompt: starting.prompt.trim() });
    setStarting(previous => ({ ...previous, prompt: '' }));
  });
  const available = agents.filter(agent => agent.available);
  if (!available.length) return null;
  const connectedStartable = available.filter(agent => agent.connected && startable.includes(agent.id));
  return <section className="settings-section">
    <div className="settings-section-heading"><h3>Coding agents</h3></div>
    {available.map(agent => <div key={agent.id} className="app-permission">
      <Toggle label={agent.name} detail={can[agent.id] ?? 'Know what it’s doing · Know when it’s done'} checked={agent.connected} disabled={busy === agent.id} onChange={on => void connect(agent.id, on)} />
      {agent.problem && <p className="field-help">{agent.problem}</p>}
      {agent.id === 'codex' && agent.connected && !agent.heard && <p className="field-help app-note">In Codex, type /hooks and trust Buntini’s hooks.</p>}
      {agent.usage !== undefined && agent.usage !== null && agent.connected && <Toggle label="Plan usage" detail="5 hours · Week" checked={agent.usage} disabled={busy === `${agent.id}-usage`} onChange={on => void usage(agent, on)} />}
    </div>)}
    {connectedStartable.length > 0 && <form className="saved-search-form agent-start" onSubmit={event => { event.preventDefault(); void start(); }}>
      <input aria-label="First message" placeholder="New session" value={starting.prompt} maxLength={8000} onChange={event => setStarting({ ...starting, prompt: event.target.value })} />
      <div>
        {connectedStartable.length > 1 && <select aria-label="Agent" value={starting.agent} onChange={event => setStarting({ ...starting, agent: event.target.value })}>{connectedStartable.map(agent => <option key={agent.id} value={agent.id}>{agent.name}</option>)}</select>}
        <select aria-label="Project" value={starting.project} onChange={event => event.target.value === '…' ? void chooseFolder() : setStarting({ ...starting, project: event.target.value })}>
          {projects.map(path => <option key={path} value={path}>{path.split('/').pop() || path}</option>)}
          <option value="…">Choose folder…</option>
        </select>
        <button className="primary-button" disabled={!starting.project || busy === 'start'}>{busy === 'start' ? <LoaderCircle size={11} className="spin" /> : <Check size={11} />} Start</button>
      </div>
    </form>}
    {week && week.stats.length > 0 && <div className="agent-week">
      <div className="agent-week-heading"><strong>{week.week}</strong><button type="button" className="secondary-button" disabled={busy === 'share'} onClick={() => void act('share', async () => { await callAgents({ op: 'shareRecap' }); })}><Share size={11} /> Share</button></div>
      <dl>{week.stats.map(stat => <div key={stat.label}><dt>{stat.label}</dt><dd>{stat.value}</dd></div>)}</dl>
    </div>}
  </section>;
}
