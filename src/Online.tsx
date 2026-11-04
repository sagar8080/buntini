import { useEffect, useState } from 'react';
import { BookOpen, CloudSun, Coins, ExternalLink, Globe2, Images, MapPin, Plane, Play, Route, ShieldCheck, ShoppingBag } from 'lucide-react';
import ModalSheet from './ModalSheet';
import { command } from './bridge';
import type { CardMap, CardPhoto, ResultCard, WebApproval } from './types';

const titles: Record<string, string> = {
  web_search: 'Search the web?', chatgpt_search: 'Ask ChatGPT?', ask_chatgpt: 'Ask ChatGPT?', weather: 'Check the weather?', currency: 'Get exchange rates?', wikipedia: 'Look it up on Wikipedia?',
  places: 'Find nearby places?', travel_time: 'Check the travel time?', ebay_search: 'Search eBay listings?', flight_search: 'Search flights?',
};
const icons = { weather: CloudSun, currency: Coins, article: BookOpen, place: MapPin, route: Route, listing: ShoppingBag, flight: Plane, photos: Images, shortcut: Play };

/** Shows exactly what one approved request will send, and to whom, before anything leaves the Mac. */
export function ApprovalSheet({ approval, approving, onCancel, onAllow, onWebInstead, onOpenChatGpt }: { approval: WebApproval; approving: boolean; onCancel: () => void; onAllow: (always?: boolean) => void; onWebInstead: () => void; onOpenChatGpt?: () => void }) {
  const web = approval.tool === 'web_search', search = approval.tool === 'chatgpt_search', asked = approval.tool === 'ask_chatgpt';
  return <ModalSheet labelledBy="approval-title">
    <ShieldCheck size={23} /><h3 id="approval-title">{titles[approval.tool] || 'Send this request?'}</h3>
    <p>{approval.summary}</p>
    <dl className="approval-sends"><div><dt>Sent to</dt><dd>{approval.service} <span>{approval.host}</span></dd></div>{approval.sends.map(field => <div key={field.label}><dt>{field.label}</dt><dd>{field.value}</dd></div>)}</dl>
    {approval.note && <p className="approval-note">{approval.note}</p>}
    <div className="approval-actions">{web && onOpenChatGpt && <button className="text-button" disabled={approving} onClick={onOpenChatGpt}>Open in ChatGPT</button>}{!web && !asked && <button className="text-button" disabled={approving} onClick={onWebInstead}>{search ? 'Use the browser' : 'Web search instead'}</button>}<button className="secondary-button" disabled={approving} onClick={onCancel}>Cancel</button>{search && <button className="secondary-button" disabled={approving} onClick={() => onAllow(true)}>Always allow</button>}<button className="primary-button" disabled={approving} autoFocus onClick={() => onAllow()}>{approving ? 'Starting…' : 'Allow once'}</button></div>
  </ModalSheet>;
}

// Maps already read this session, so a thread doesn't fetch them again as it re-renders.
const pictures = new Map<string, string>();

/** The map drawn for these results; opens the same view in Maps. */
function MapPicture({ map, label, onOpen }: { map: CardMap; label: string; onOpen: (url: string) => void }) {
  const [source, setSource] = useState(pictures.get(map.id) || '');
  useEffect(() => {
    if (pictures.has(map.id)) return;
    let current = true;
    command<string>('map_picture', { id: map.id }).then(data => { if (data) pictures.set(map.id, data); if (current) setSource(data); }).catch(() => {});
    return () => { current = false; };
  }, [map.id]);
  if (!source) return null;
  return <button type="button" className="result-map" aria-label={`Open ${label} in Maps`} onClick={() => onOpen(map.url)}><img src={source} alt="" /></button>;
}

/** One photo's thumbnail; opens it in Photos. */
function PhotoThumb({ photo }: { photo: CardPhoto }) {
  const [source, setSource] = useState(pictures.get(photo.thumb) || '');
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!photo.thumb || pictures.has(photo.thumb)) return;
    let current = true;
    command<string>('photo_thumbnail', { id: photo.thumb }).then(data => { if (data) pictures.set(photo.thumb, data); if (current) setSource(data); }).catch(() => {});
    return () => { current = false; };
  }, [photo.thumb]);
  const taken = photo.taken ? new Date(photo.taken).toLocaleDateString(undefined, { day: 'numeric', month: 'short', year: 'numeric' }) : '';
  return <button type="button" className={`photo-thumb ${failed ? 'is-failed' : ''}`} aria-label={`Open the ${photo.video ? 'video' : 'photo'}${taken ? ` from ${taken}` : ''} in Photos`} title={taken}
    onClick={() => { setFailed(false); command('show_photo', { id: photo.id }).catch(() => setFailed(true)); }}>
    {source ? <img src={source} alt="" /> : <Images size={16} />}
    {photo.video && <span className="photo-video"><Play size={10} /></span>}
  </button>;
}

/** Opens Shortcuts on a new shortcut, when none of the user's fits. */
function NewShortcut({ label }: { label: string }) {
  const [failed, setFailed] = useState(false);
  return <div className="row-actions"><button type="button" className="secondary-button" onClick={() => { setFailed(false); command('new_shortcut').catch(() => setFailed(true)); }}>{label}</button>{failed && <span className="field-help">Shortcuts didn’t open.</span>}</div>;
}

/** Numbered cards matching the [n] citations in the answer. */
export function ResultCards({ cards, onOpen }: { cards: ResultCard[]; onOpen: (url: string) => void }) {
  if (cards.length === 1 && cards[0].kind === 'shortcut') return <NewShortcut label={cards[0].title} />;
  const mapped = cards.find(card => card.map);
  return <div className="result-cards"><div className="section-label">Live results</div>
    {mapped?.map && <MapPicture map={mapped.map} label={mapped.kind === 'route' ? mapped.subtitle : mapped.subtitle.split(' · ')[0]} onOpen={onOpen} />}
    {cards.map((card, index) => {
      const Icon = icons[card.kind] || Globe2;
      return <article key={`${card.title}-${index}`} className={`result-card result-card--${card.kind}`}>
        <header><span className="source-number">{index + 1}</span><Icon size={15} /><div><strong>{card.title}</strong>{card.subtitle && <small>{card.subtitle}</small>}</div>
          {card.url && <a href={card.url} target="_blank" rel="noreferrer" aria-label={`Open ${card.title}`} onClick={event => { event.preventDefault(); onOpen(card.url!); }}><ExternalLink size={13} /></a>}</header>
        {!!card.photos?.length && <div className="photo-grid">{card.photos.map(photo => <PhotoThumb key={photo.id} photo={photo} />)}</div>}
        {card.fields.length > 0 && <dl>{card.fields.map(field => <div key={field.label}><dt>{field.label}</dt><dd>{field.value}</dd></div>)}</dl>}
        <footer>{card.attribution}</footer>
      </article>;
    })}
  </div>;
}
