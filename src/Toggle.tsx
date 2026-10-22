export default function Toggle({ label, detail, checked, disabled = false, onChange }: { label: string; detail?: string; checked: boolean; disabled?: boolean; onChange: (value: boolean) => void }) {
  return <label className={`toggle-row ${disabled ? 'is-disabled' : ''}`}><span>{label}{detail && <small>{detail}</small>}</span><input type="checkbox" checked={checked} disabled={disabled} onChange={event => onChange(event.target.checked)} /><span className="toggle-track" aria-hidden="true"><span /></span></label>;
}
