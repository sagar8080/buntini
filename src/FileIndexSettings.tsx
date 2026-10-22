import { useEffect, useState } from 'react';
import { FolderSync, LoaderCircle } from 'lucide-react';
import { command, desktop, errorMessage } from './bridge';

type IndexStatus = { files: number; truncated: boolean; indexedAt?: string };
export default function FileIndexSettings({ roots, onError }: { roots: string[]; onError: (error: string) => void }) {
  const [status, setStatus] = useState<IndexStatus | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  useEffect(() => {
    if (!desktop) return;
    let active = true;
    void command<IndexStatus>('file_index_status').then(result => { if (active) setStatus(result); }).catch(() => {});
    return () => { active = false; };
  }, [roots.join('\n')]);
  return <div className="file-index-controls"><button type="button" className="secondary-button" disabled={refreshing || !roots.length} onClick={async () => { setRefreshing(true); try { setStatus(await command<IndexStatus>('refresh_file_index')); } catch (error) { onError(errorMessage(error)); } finally { setRefreshing(false); } }}>{refreshing ? <LoaderCircle size={12} className="spin" /> : <FolderSync size={12} />}{refreshing ? 'Indexing…' : 'Refresh index'}</button>{refreshing && <button type="button" className="text-button" onClick={async () => { try { await command('cancel_file_index'); } catch (error) { onError(errorMessage(error)); } }}>Stop</button>}{status?.indexedAt && <p title={new Date(status.indexedAt).toLocaleString()}>{status.files} files indexed{status.truncated ? ' · limit reached' : ''}</p>}</div>;
}
