import { BookOpen, CloudSun, Coins, ExternalLink, Globe2, MapPin, Plane, ShieldCheck, ShoppingBag } from 'lucide-react';
import ModalSheet from './ModalSheet';
import type { ResultCard, WebApproval } from './types';

const titles: Record<string, string> = {
  web_search: 'Search the web?', weather: 'Check the weather?', currency: 'Get exchange rates?', wikipedia: 'Look it up on Wikipedia?',
  places: 'Find nearby places?', ebay_search: 'Search eBay listings?', flight_search: 'Search flights?',
};
const icons = { weather: CloudSun, currency: Coins, article: BookOpen, place: MapPin, listing: ShoppingBag, flight: Plane };

/** Shows exactly what one approved request will send, and to whom, before anything leaves the Mac. */
export function ApprovalSheet({ approval, approving, onCancel, onAllow, onWebInstead }: { approval: WebApproval; approving: boolean; onCancel: () => void; onAllow: () => void; onWebInstead: () => void }) {
  const web = approval.tool === 'web_search';
  return <ModalSheet labelledBy="approval-title">
    <ShieldCheck size={23} /><h3 id="approval-title">{titles[approval.tool] || 'Send this request?'}</h3>
    <p>{approval.summary}</p>
    <dl className="approval-sends"><div><dt>Sent to</dt><dd>{approval.service} <span>{approval.host}</span></dd></div>{approval.sends.map(field => <div key={field.label}><dt>{field.label}</dt><dd>{field.value}</dd></div>)}</dl>
    {approval.note && <p className="approval-note">{approval.note}</p>}
    <div>{!web && <button className="text-button" disabled={approving} onClick={onWebInstead}>Web search instead</button>}<button className="secondary-button" disabled={approving} onClick={onCancel}>Cancel</button><button className="primary-button" disabled={approving} autoFocus onClick={onAllow}>{approving ? 'Starting…' : 'Allow once'}</button></div>
  </ModalSheet>;
}

/** Numbered cards matching the [n] citations in the answer. */
export function ResultCards({ cards, onOpen }: { cards: ResultCard[]; onOpen: (url: string) => void }) {
  return <div className="result-cards"><div className="section-label">Live results</div>
    {cards.map((card, index) => {
      const Icon = icons[card.kind] || Globe2;
      return <article key={`${card.title}-${index}`} className={`result-card result-card--${card.kind}`}>
        <header><span className="source-number">{index + 1}</span><Icon size={15} /><div><strong>{card.title}</strong>{card.subtitle && <small>{card.subtitle}</small>}</div>
          {card.url && <a href={card.url} target="_blank" rel="noreferrer" aria-label={`Open ${card.title}`} onClick={event => { event.preventDefault(); onOpen(card.url!); }}><ExternalLink size={13} /></a>}</header>
        {card.fields.length > 0 && <dl>{card.fields.map(field => <div key={field.label}><dt>{field.label}</dt><dd>{field.value}</dd></div>)}</dl>}
        <footer>{card.attribution}</footer>
      </article>;
    })}
  </div>;
}
