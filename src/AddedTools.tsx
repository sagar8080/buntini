import { useEffect, useState } from 'react';
import { ChevronDown, FileJson, LoaderCircle, Plug, Plus } from 'lucide-react';
import Toggle from './Toggle';
import { command, desktop, errorMessage } from './bridge';

/** A local MCP server the user added. Environment values never come back from the app, only their names. */
export interface AddedTool { id: string; name: string; command: string; env: string[]; tools: { name: string; description: string; safe: boolean; unusable?: string | null }[] }
interface Found { name: string; command: string; env: string[]; unsupported?: string | null }

function Server({ server, busy, onSafe, onReconnect, onRemove }: { server: AddedTool; busy: boolean; onSafe: (tool: string, safe: boolean) => void; onReconnect: () => void; onRemove: () => void }) {
  const [open, setOpen] = useState(false);
  return <div className={`service-row ${open ? 'is-open' : ''}`}>
    <button type="button" className="service-summary" aria-expanded={open} onClick={() => setOpen(!open)}><Plug size={18} /><span><strong>{server.name}</strong><small title={server.command}>{server.command}</small></span><ChevronDown size={14} /></button>
    {open && <div className="service-body">
      {server.env.length > 0 && <p className="field-help">Uses {server.env.join(', ')}, kept on this Mac.</p>}
      {server.tools.map(tool => tool.unusable
        ? <p className="field-help" key={tool.name}>{tool.name}: {tool.unusable}</p>
        : <Toggle key={tool.name} label={tool.name} detail={tool.safe ? 'Runs without asking' : 'Asks you first'} checked={tool.safe} disabled={busy} onChange={safe => onSafe(tool.name, safe)} />)}
      <p className="field-help">Turn a tool on only if it just reads. What it says about itself isn’t checked.</p>
      <div className="service-actions">
        <button type="button" className="secondary-button" disabled={busy} onClick={onReconnect}>{busy ? <LoaderCircle size={13} className="spin" /> : 'Reconnect'}</button>
        <button type="button" className="text-button danger" disabled={busy} onClick={onRemove}>Remove</button>
      </div>
    </div>}
  </div>;
}

/** Tools you added: local MCP servers, started from a pasted command or an MCP config file you pick. */
export default function AddedTools({ onError }: { onError: (error: string) => void }) {
  const [servers, setServers] = useState<AddedTool[]>([]);
  const [line, setLine] = useState('');
  const [found, setFound] = useState<Found[] | null>(null);
  const [busy, setBusy] = useState('');
  useEffect(() => {
    if (!desktop) return;
    let active = true;
    void command<AddedTool[]>('added_tools').then(list => { if (active) setServers(list); }).catch(() => {});
    return () => { active = false; };
  }, []);
  const replace = (server: AddedTool) => setServers(list => list.some(item => item.id === server.id) ? list.map(item => item.id === server.id ? server : item) : [...list, server]);
  async function run(key: string, work: () => Promise<void>) {
    setBusy(key);
    try { await work(); } catch (error) { onError(errorMessage(error)); } finally { setBusy(''); }
  }
  return <section className="settings-section">
    <div className="settings-section-heading"><h3>Tools you added</h3></div>
    {servers.map(server => <Server key={server.id} server={server} busy={busy === server.id}
      onSafe={(tool, safe) => void run(server.id, async () => replace(await command<AddedTool>('set_tool_safe', { id: server.id, tool, safe })))}
      onReconnect={() => void run(server.id, async () => replace(await command<AddedTool>('reconnect_tool_server', { id: server.id })))}
      onRemove={() => void run(server.id, async () => { await command('remove_tool_server', { id: server.id }); setServers(list => list.filter(item => item.id !== server.id)); })} />)}
    <div className="key-input"><Plug size={14} /><input type="text" autoComplete="off" spellCheck={false} aria-label="Command that starts an MCP server" placeholder="npx -y @modelcontextprotocol/server-github" value={line} onChange={event => setLine(event.target.value)} /></div>
    <div className="row-actions">
      <button type="button" className="secondary-button" disabled={!line.trim() || !!busy} onClick={() => void run('add', async () => { replace(await command<AddedTool>('add_tool_server', { command: line.trim() })); setLine(''); })}>{busy === 'add' ? <LoaderCircle size={13} className="spin" /> : <Plus size={13} />}Add</button>
      <button type="button" className="text-button" disabled={!!busy} onClick={() => void run('pick', async () => setFound(await command<Found[] | null>('pick_tool_config')))}><FileJson size={12} />Add from a config file</button>
    </div>
    {found?.map(item => <div className="root-item" key={item.name}><span title={item.command}>{item.name}{item.unsupported ? ` · ${item.unsupported}` : ''}</span>
      {!item.unsupported && !servers.some(server => server.name.toLowerCase() === item.name.toLowerCase()) && <button type="button" aria-label={`Add ${item.name}`} title="Add" disabled={!!busy} onClick={() => void run(`config:${item.name}`, async () => replace(await command<AddedTool>('add_tool_server_from_config', { entry: item.name })))}>{busy === `config:${item.name}` ? <LoaderCircle size={13} className="spin" /> : <Plus size={14} />}</button>}
    </div>)}
    <p className="field-help">Paste the command that starts a local MCP server, with any values it needs first, like TOKEN=… npx …. Then ask “use {servers[0]?.name ?? 'GitHub'} to …”. I ask before each call.</p>
  </section>;
}
