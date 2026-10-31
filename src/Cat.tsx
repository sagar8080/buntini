import type { CatCoat, PetLook } from './types';
import { eyeColors } from './petOptions';
import { luminance, mix } from './themes';

export interface Coat {
  base: string; shade: string; outline: string; belly: string; ear: string; nose: string; whisker: string;
  marks?: string; patchA?: string; patchB?: string; point?: string; blush: number;
}

export const palettes: Record<CatCoat, Coat> = {
  ginger: { base: '#F6AA5E', shade: '#E2843D', outline: '#9A5126', belly: '#FFF1DC', ear: '#F7B3A6', nose: '#F08A98', whisker: '#9A5126', marks: '#D46F2D', blush: .38 },
  gray: { base: '#A9B1BE', shade: '#8A93A2', outline: '#4E5563', belly: '#F2F3F6', ear: '#F2B8B5', nose: '#E58C9A', whisker: '#4E5563', marks: '#6E7787', blush: .38 },
  black: { base: '#3E3B45', shade: '#2B2930', outline: '#141217', belly: '#57515F', ear: '#C98A93', nose: '#7A505D', whisker: '#CFCAD6', blush: .24 },
  white: { base: '#FFFDF8', shade: '#ECE5DB', outline: '#A99E92', belly: '#FFFFFF', ear: '#F7B9B6', nose: '#F08A98', whisker: '#B3A79B', blush: .42 },
  tuxedo: { base: '#37343D', shade: '#27252C', outline: '#141217', belly: '#FFFFFF', ear: '#C98A93', nose: '#F09AA6', whisker: '#D8D3DE', blush: .3 },
  calico: { base: '#FFFBF4', shade: '#EDE4D8', outline: '#9C8D7E', belly: '#FFFFFF', ear: '#F7B9B6', nose: '#F08A98', whisker: '#A8998A', patchA: '#EF9B4E', patchB: '#433D44', blush: .4 },
  siamese: { base: '#F6ECDC', shade: '#E4D3BB', outline: '#7A5A45', belly: '#FBF5EC', ear: '#8A6352', nose: '#5E4032', whisker: '#7A5A45', point: '#5E4032', blush: .3 },
};

const flowers: [number, number][] = [[66, 69], [77, 62], [90, 59], [103, 62], [114, 69]];
const ink = '#2A2230';

/* Seated, round-headed kitten. Coat, eyes and accessory come from the saved look;
   state reactions are CSS-driven so the SVG stays static and cheap to render.
   Light comes from the upper left. Shading is gradients only (no filters), so a
   moving cat costs no more to draw than a still one. */
export type CatPose = 'sit' | 'walk' | 'leap';

export default function Cat({ id, look, pose = 'sit' }: { id: string; look: PetLook; pose?: CatPose }) {
  const idPrefix = id;
  const url = (name: string) => `url(#${id}-${name})`;
  const coat = palettes[look.coat];
  const accent = look.accent;
  const tabby = look.coat === 'ginger' || look.coat === 'gray';
  const leftIris = look.eyes === 'odd' ? eyeColors.blue : eyeColors[look.eyes];
  const rightIris = look.eyes === 'odd' ? eyeColors.amber : eyeColors[look.eyes];
  const tail = coat.point ?? coat.patchB ?? coat.base;
  const paws = look.coat === 'tuxedo' ? coat.belly : coat.point ?? coat.base;
  const leftEar = coat.point ?? coat.patchA ?? coat.base;
  const rightEar = coat.point ?? coat.patchB ?? coat.base;
  // Shadow tones lean toward the outline rather than grey, so a ginger shades to rust.
  const deep = mix(coat.shade, coat.outline, .3);
  const tuft = mix(coat.belly, '#ffffff', .5);
  // Far legs sit in shadow but keep their colour: a Siamese's points, a tuxedo's white socks.
  const farLeg = coat.point ? mix(coat.point, coat.outline, .25) : coat.shade;
  const farPaw = mix(paws, coat.outline, .25);
  const lids = look.coat === 'black' || look.coat === 'tuxedo' ? '#F4E9DA' : look.coat === 'siamese' ? '#2E1F18' : coat.outline;
  // Outlines lighten toward the light (`edge`), so the cat reads as lit rather than cut out.
  // Pale coats lighten them only a little, so they keep an edge against white windows.
  const edge = url('edge'), edgeLift = luminance(coat.base) > .7 ? .12 : .32;
  const line = { stroke: edge, strokeWidth: 1.8, strokeLinejoin: 'round' as const };
  // The outline layer is drawn 3.6 wide under the fills, which shows the same 1.8 line as `line`.
  // Legs keep a flat outline: their joints move them far from where the gradient falls.
  const paint = (outline: boolean, color: string, tone = edge) => outline ? { fill: tone, stroke: tone, strokeWidth: 3.6, strokeLinejoin: 'round' as const } : { fill: color };

  // A walking leg is jointed: the upper leg turns at the hip (x, 131), the lower leg at
  // the knee and the paw at the ankle, so a step can plant, push, fold and reach.
  // Each joint sits at its group's origin; CSS animates the three groups (styles.css).
  // Legs are drawn in two layers, outline underneath and fill on top (see `paint`), so
  // the joints, and the near legs' tops, merge into one silhouette without seams.
  const leg = (x: number, id: 'nf' | 'nb' | 'ff' | 'fb', end: 'front' | 'back', far: boolean, outline: boolean) => {
    const fill = `url(#${idPrefix}-leg${far ? '-far' : ''})`;
    const pawFill = far ? farPaw : `url(#${idPrefix}-paw)`;
    const layer = () => {
      return <g className={`walk-leg leg-${id} ${end}`}>
        <path d="M-6.6 -4C-6.6 -7 6.6 -7 6.6 -4L4.9 13.2C4.6 16.2 -4.6 16.2 -4.9 13.2Z" {...paint(outline, fill, coat.outline)} />
        <g transform="translate(0 14)"><g className="walk-shin">
          <path d="M-4.6 -2.6C-4.6 -4.6 4.6 -4.6 4.6 -2.6L3.6 12.2C3.5 14.4 -3.5 14.4 -3.6 12.2Z" {...paint(outline, fill, coat.outline)} />
          <g transform="translate(0 13)"><g className="walk-paw">
            <path d="M-7.4 1.8C-7.4 -1.2 -4.6 -3 -1.2 -3C2.6 -3 4.8 -1.2 4.8 1.8C4.8 4.3 2.4 5.4 -1.4 5.4C-5.1 5.4 -7.4 4.3 -7.4 1.8Z" {...paint(outline, pawFill, coat.outline)} />
            {!outline && !far && <path d="M-4.6 2.6V4.6M-1.4 2.8V5" stroke={coat.outline} strokeWidth="1" strokeLinecap="round" strokeOpacity=".5" />}
          </g></g>
        </g></g>
      </g>;
    };
    return <g transform={`translate(${x} 131)`} key={id}>{layer()}</g>;
  };
  const torso = 'M96 128C96 116 106 110 120 110H134C146 110 153 117 153 128C153 139 146 145 134 145H112C102 145 96 139 96 128Z';
  const body = 'M66 122C57 134 56 150 63 158C69 164 111 164 117 158C124 150 123 134 114 122Z';
  const head = 'M52 98C49 76 66 58 90 58C114 58 131 76 128 98L134 103L127 105L131 111L123 110C117 122 105 128 90 128C75 128 63 122 57 110L49 111L53 105L46 103Z';
  const sitTail = 'M110 154C132 160 150 150 151 128C152 116 146 106 138 104';
  const tailBase = 'M150 122C164 118 170 104 166 90', tailTip = 'M166 90C164 83 159 81 157 85';
  const outerEar = 'M57 82C53 68 52 54 55 45C56 41 60 40 63 42C71 48 78 55 84 63Z';
  const ear = (fill: string) => <>
    <path d={outerEar} fill={fill} {...line} />
    <path d={outerEar} fill={url('ear-tip')} />
    <path d="M61 73C59 63 59 55 60 49C66 53 71 58 76 63Z" fill={url('inner-ear')} />
    <path d="M64.5 70Q63 64 63.6 58M68.5 67Q66.6 62 66.4 56.6M72.6 64.6Q70 61 69.4 58" stroke={tuft} strokeWidth="1" strokeLinecap="round" strokeOpacity=".6" />
  </>;
  // `side` is -1 for the left eye and 1 for the right, which can have its own colour.
  const eye = (cx: number, side: -1 | 1, far = false) => <g className={far ? 'cat-eye cat-eye-far' : 'cat-eye'}>
    <ellipse cx={cx} cy="94" rx="9" ry="10" fill={url(side < 0 ? 'iris-left' : 'iris-right')} />
    <path d={`M${cx - 8.9} 92.6A9 10 0 0 1 ${cx + 8.9} 92.6Q${cx} 83.4 ${cx - 8.9} 92.6Z`} fill="#1E1724" fillOpacity=".24" />
    <g className="cat-look"><g className="cat-pupil">
      <ellipse cx={cx} cy="94.5" rx="5.1" ry="7.3" fill="#1E1724" />
      <ellipse cx={cx - 2.9} cy="89.8" rx="2.9" ry="3.2" fill="#fff" />
      <circle cx={cx + 3.3} cy="98.8" r="1.4" fill="#fff" fillOpacity=".85" />
    </g></g>
    {/* The upper lid waits above the eye until a mood lowers it (--lid) or tilts it
        (--lid-tilt): soft when content, heavy when drowsy, angled when grumpy. */}
    <g clipPath={url(side < 0 ? 'eye-left' : 'eye-right')}><g className={`cat-lid cat-lid-${side < 0 ? 'left' : 'right'}`}>
      <path d={`M${cx - 11} 70H${cx + 11}V92Q${cx} 99 ${cx - 11} 92Z`} fill={url('head')} />
      <path d={`M${cx + 11} 92Q${cx} 99 ${cx - 11} 92`} stroke={ink} strokeWidth="2.2" strokeLinecap="round" />
    </g></g>
    <ellipse cx={cx} cy="94" rx="9" ry="10" stroke={ink} strokeWidth="1.2" />
    <path d={`M${cx - 8.5} 90.6A9 10 0 0 1 ${cx + 8.5} 90.6`} stroke={ink} strokeWidth="2.2" strokeLinecap="round" />
  </g>;
  const star = (x: number, y: number, r: number) => `M${x} ${y - r}Q${x} ${y} ${x + r} ${y}Q${x} ${y} ${x} ${y + r}Q${x} ${y} ${x - r} ${y}Q${x} ${y} ${x} ${y - r}Z`;
  const iris = (name: string, color: string) => <radialGradient id={`${id}-${name}`} cx=".5" cy=".66" r=".62">
    <stop stopColor={mix(color, '#ffffff', .55)} /><stop offset=".45" stopColor={color} /><stop offset=".86" stopColor={mix(color, ink, .35)} /><stop offset="1" stopColor={mix(color, ink, .6)} />
  </radialGradient>;

  return <svg viewBox="0 0 180 180" fill="none" xmlns="http://www.w3.org/2000/svg" className={`cat cat--${pose}`}>
    <defs>
      <linearGradient id={`${id}-edge`} x1="62" y1="52" x2="124" y2="160" gradientUnits="userSpaceOnUse"><stop stopColor={mix(coat.outline, coat.base, edgeLift)} /><stop offset=".55" stopColor={coat.outline} /></linearGradient>
      {/* Head and seated body: a sheen at the top left, then form shading laid over the
          markings so patches and stripes curve with the body too. */}
      <radialGradient id={`${id}-head`} cx="80" cy="72" r="44" gradientUnits="userSpaceOnUse"><stop stopColor={mix(coat.base, '#ffffff', .26)} /><stop offset="1" stopColor={coat.base} /></radialGradient>
      <radialGradient id={`${id}-head-form`} cx="86" cy="86" r="46" gradientUnits="userSpaceOnUse"><stop offset=".6" stopColor={deep} stopOpacity="0" /><stop offset="1" stopColor={deep} stopOpacity=".68" /></radialGradient>
      {/* The top of the body sits in the head's shadow. */}
      <linearGradient id={`${id}-body`} x1="0" y1="122" x2="0" y2="164" gradientUnits="userSpaceOnUse"><stop stopColor={mix(coat.base, coat.outline, .25)} /><stop offset=".26" stopColor={coat.base} /><stop offset="1" stopColor={coat.base} /></linearGradient>
      <radialGradient id={`${id}-body-form`} cx="84" cy="140" r="40" gradientUnits="userSpaceOnUse"><stop offset=".5" stopColor={deep} stopOpacity="0" /><stop offset="1" stopColor={deep} stopOpacity=".72" /></radialGradient>
      <linearGradient id={`${id}-bib`} x1="0" y1="126" x2="0" y2="160" gradientUnits="userSpaceOnUse"><stop stopColor={mix(coat.belly, coat.shade, .6)} /><stop offset=".28" stopColor={coat.belly} /></linearGradient>
      <linearGradient id={`${id}-ruff`} x1="0" y1="124.5" x2="0" y2="129" gradientUnits="userSpaceOnUse"><stop stopColor={coat.belly} stopOpacity="0" /><stop offset="1" stopColor={coat.belly} /></linearGradient>
      <radialGradient id={`${id}-muzzle`} cx=".5" cy=".5" r=".58"><stop stopColor={coat.belly} /><stop offset=".74" stopColor={coat.belly} /><stop offset="1" stopColor={coat.belly} stopOpacity="0" /></radialGradient>
      <radialGradient id={`${id}-mask`} cy=".56"><stop stopColor={coat.point} stopOpacity=".95" /><stop offset=".4" stopColor={coat.point} stopOpacity=".82" /><stop offset=".75" stopColor={coat.point} stopOpacity=".38" /><stop offset="1" stopColor={coat.point} stopOpacity="0" /></radialGradient>
      <radialGradient id={`${id}-blush`}><stop stopColor="#FF8DA1" stopOpacity={Math.min(1, coat.blush * 1.6)} /><stop offset="1" stopColor="#FF8DA1" stopOpacity="0" /></radialGradient>
      {iris('iris-left', leftIris)}{iris('iris-right', rightIris)}
      <clipPath id={`${id}-eye-left`}><ellipse cx="74" cy="94" rx="9" ry="10" /></clipPath>
      <clipPath id={`${id}-eye-right`}><ellipse cx="106" cy="94" rx="9" ry="10" /></clipPath>
      <linearGradient id={`${id}-nose`} x1="0" y1="0" x2="0" y2="1"><stop stopColor={mix(coat.nose, '#ffffff', .3)} /><stop offset="1" stopColor={coat.nose} /></linearGradient>
      <linearGradient id={`${id}-ear-tip`} x1="58" y1="44" x2="70" y2="72" gradientUnits="userSpaceOnUse"><stop stopColor={coat.outline} stopOpacity=".28" /><stop offset="1" stopColor={coat.outline} stopOpacity="0" /></linearGradient>
      <linearGradient id={`${id}-inner-ear`} x1="0" y1="49" x2="0" y2="73" gradientUnits="userSpaceOnUse"><stop stopColor={mix(coat.ear, coat.outline, .3)} /><stop offset="1" stopColor={coat.ear} /></linearGradient>
      <linearGradient id={`${id}-paw`} x1="0" y1="0" x2="0" y2="1"><stop stopColor={mix(paws, '#ffffff', .25)} /><stop offset=".5" stopColor={paws} /><stop offset="1" stopColor={mix(paws, coat.outline, .2)} /></linearGradient>
      {/* Tails darken where they tuck in behind the body. */}
      <linearGradient id={`${id}-tail`} x1="112" y1="156" x2="140" y2="104" gradientUnits="userSpaceOnUse"><stop stopColor={mix(tail, coat.outline, .35)} /><stop offset=".4" stopColor={tail} /><stop offset="1" stopColor={mix(tail, '#ffffff', .1)} /></linearGradient>
      <linearGradient id={`${id}-walk-tail`} x1="150" y1="124" x2="160" y2="84" gradientUnits="userSpaceOnUse"><stop stopColor={mix(tail, coat.outline, .25)} /><stop offset=".45" stopColor={tail} /><stop offset="1" stopColor={mix(tail, '#ffffff', .1)} /></linearGradient>
      <linearGradient id={`${id}-coat`} x1="0" y1="106" x2="0" y2="146" gradientUnits="userSpaceOnUse"><stop stopColor={mix(coat.base, '#ffffff', .16)} /><stop offset=".35" stopColor={coat.base} /><stop offset=".75" stopColor={coat.base} /><stop offset="1" stopColor={coat.shade} /></linearGradient>
      <radialGradient id={`${id}-torso-form`} cx="118" cy="116" r="38" gradientUnits="userSpaceOnUse"><stop offset=".55" stopColor={deep} stopOpacity="0" /><stop offset="1" stopColor={deep} stopOpacity=".6" /></radialGradient>
      <radialGradient id={`${id}-neck`} cx="112" cy="120" r="20" gradientUnits="userSpaceOnUse"><stop stopColor={coat.outline} stopOpacity=".3" /><stop offset="1" stopColor={coat.outline} stopOpacity="0" /></radialGradient>
      <linearGradient id={`${id}-leg`} x1="0" y1="0" x2="1" y2="0"><stop stopColor={coat.point ?? coat.base} /><stop offset=".65" stopColor={coat.point ?? coat.base} /><stop offset="1" stopColor={coat.shade} /></linearGradient>
      <linearGradient id={`${id}-leg-far`} x1="0" y1="0" x2="1" y2="0"><stop stopColor={farLeg} /><stop offset="1" stopColor={mix(farLeg, coat.outline, .3)} /></linearGradient>
      <linearGradient id={`${id}-belly`} x1="100" y1="0" x2="136" y2="0" gradientUnits="userSpaceOnUse"><stop stopColor={coat.belly} /><stop offset=".4" stopColor={coat.belly} /><stop offset="1" stopColor={coat.belly} stopOpacity="0" /></linearGradient>
      <radialGradient id={`${id}-shadow`}><stop stopColor="#1D1720" stopOpacity=".3" /><stop offset=".55" stopColor="#1D1720" stopOpacity=".15" /><stop offset="1" stopColor="#1D1720" stopOpacity="0" /></radialGradient>
      <radialGradient id={`${id}-yarn`} cx=".38" cy=".32" r=".75"><stop stopColor={mix(accent, '#ffffff', .35)} /><stop offset=".5" stopColor={accent} /><stop offset="1" stopColor={mix(accent, '#1D1720', .35)} /></radialGradient>
      <radialGradient id={`${id}-bell`} cx=".36" cy=".3" r=".75"><stop stopColor="#FFF1B0" /><stop offset=".45" stopColor="#F6C945" /><stop offset="1" stopColor="#C9921F" /></radialGradient>
      <linearGradient id={`${id}-heart`} x1="0" y1="0" x2="0" y2="1"><stop stopColor="#FF9BB0" /><stop offset="1" stopColor="#F0577A" /></linearGradient>
    </defs>
    {/* Soft shadows: a wide one under the cat and a darker one where the paws touch down. */}
    <g className="pet-shadow cat-sit-shadow"><ellipse cx="90" cy="165" rx="44" ry="7" fill={url('shadow')} /><ellipse cx="90" cy="164" rx="34" ry="3.6" fill={url('shadow')} /></g>
    <g className="pet-shadow cat-walk-shadow"><ellipse cx="114" cy="165" rx="52" ry="7" fill={url('shadow')} /></g>
    <g className="pet-body">
      {/* Seated body. While walking it fades out and the walking body below fades in;
          the head never changes, so switching poses reads as the cat getting up. */}
      <g className="cat-sit">
      <g className="cat-tail">
        <path d={sitTail} stroke={edge} strokeWidth="13.5" strokeLinecap="round" />
        <path d={sitTail} stroke={url('tail')} strokeWidth="10" strokeLinecap="round" />
        {tabby && <path d={sitTail} stroke={coat.marks} strokeWidth="10" strokeDasharray="4 7" strokeDashoffset="-8" />}
      </g>

      {/* Back paws peek out beside the front ones. */}
      <g className="cat-paw cat-hind-paw">
        <path d="M56 160C56 156.6 59.4 155 63.5 155C67.6 155 70 156.6 70 160C70 162.6 67.4 164 63 164C59 164 56 162.6 56 160Z" fill={url('paw')} {...line} />
        <path d="M124 160C124 156.6 120.6 155 116.5 155C112.4 155 110 156.6 110 160C110 162.6 112.6 164 117 164C121 164 124 162.6 124 160Z" fill={url('paw')} {...line} />
      </g>
      <path d={body} fill={url('body')} />
      {look.coat === 'calico' && <g>
        <path d="M66 124C60 134 59 146 62 154C68 152 72 144 72 134C71 129 69 126 66 124Z" fill={coat.patchA} />
        <path d="M114 124C120 134 121 144 118 152C112 150 108 142 108 134C109 129 111 126 114 124Z" fill={coat.patchB} />
      </g>}
      <path d="M79 126C74 136 74 150 79 158H101C106 150 106 136 101 126C95 130 85 130 79 126Z" fill={url('bib')} />
      {tabby && <path d="M63 134C67 136 70 135 72 132M61 143C66 145 69 144 71 141M62 152C66 153 69 152 70 150M117 134C113 136 110 135 108 132M119 143C114 145 111 144 109 141M118 152C114 153 111 152 110 150" stroke={coat.marks} strokeWidth="2.4" strokeLinecap="round" />}
      {/* The curve of each haunch. */}
      <path d="M71 154C66.4 150.4 64.6 143.6 66.6 136.4M109 154C113.6 150.4 115.4 143.6 113.4 136.4" stroke={coat.outline} strokeOpacity=".4" strokeWidth="1.4" strokeLinecap="round" />
      <path d={body} fill={url('body-form')} />
      <path d={body} {...line} />
      <g className="cat-paw cat-paw-left">
        <path d="M70 156C70 151 74 149 79 149C84 149 87 151 87 156C87 161 84 163 79 163C74 163 70 161 70 156Z" fill={url('paw')} {...line} />
        <path d="M76 158V161M82 158V161" stroke={coat.outline} strokeWidth="1.2" strokeLinecap="round" strokeOpacity=".55" />
      </g>
      <g className="cat-paw cat-paw-right">
        <path d="M93 156C93 151 97 149 102 149C107 149 110 151 110 156C110 161 107 163 102 163C97 163 93 161 93 156Z" fill={url('paw')} {...line} />
        <path d="M99 158V161M105 158V161" stroke={coat.outline} strokeWidth="1.2" strokeLinecap="round" strokeOpacity=".55" />
      </g>
      </g>

      {/* Walking body, side-on behind the head: far legs, tail, torso, near legs. */}
      <g className="cat-walkbody">
        {[true, false].map(outline => <g key={String(outline)}>{leg(111, 'ff', 'front', true, outline)}{leg(146, 'fb', 'back', true, outline)}</g>)}
        {/* Tail, near legs, neck and torso share one outline: every outline goes down
            first, then every fill, so the parts merge into a single body with no seams. */}
        {[true, false].map(outline => <g key={String(outline)}>
          <g className="walk-tail">
            <path d={tailBase} stroke={outline ? edge : url('walk-tail')} strokeWidth={outline ? 13 : 9.5} strokeLinecap="round" />
            <g className="walk-tail-tip"><path d={tailTip} stroke={outline ? edge : mix(tail, '#ffffff', .1)} strokeWidth={outline ? 13 : 9.5} strokeLinecap="round" /></g>
          </g>
          {leg(101, 'nf', 'front', false, outline)}
          {leg(136, 'nb', 'back', false, outline)}
          <path d="M86 110C94 105 106 108 113 116L115 132C109 140 98 141 92 134C86 127 83 117 86 110Z" {...paint(outline, coat.base)} />
          <path d={torso} {...paint(outline, url('coat'))} />
        </g>)}
        {tabby && <g className="walk-tail">
          <path d={tailBase} stroke={coat.marks} strokeWidth="9.5" strokeDasharray="4 6" />
          <g className="walk-tail-tip"><path d={tailTip} stroke={coat.marks} strokeWidth="9.5" strokeDasharray="4 5" strokeDashoffset="-2" /></g>
        </g>}
        {look.coat === 'calico' && <g>
          <path d="M116 111C124 109 132 111 135 116C129 121 119 121 113 117C112 114 113 112 116 111Z" fill={coat.patchA} />
          <path d="M140 112C148 114 153 120 153 128C147 128 141 124 138 118Z" fill={coat.patchB} />
        </g>}
        {/* A cream chest that fades along the belly, like the seated cat's front. */}
        <path d="M96 122C101 119 108 122 110 129C111 133 110 137 108 141C116 143 126 143 136 141C134 144 133 145 130 145H112C104 145 97 140 96 122Z" fill={url('belly')} />
        {look.coat === 'tuxedo' && <path d="M96 128C96 138 102 144 110 145C113 137 112 124 105 117C100 119 96 123 96 128Z" fill={coat.belly} />}
        {tabby && <path d="M118 111C119 116 118 120 116 123M128 111C129 116 128 120 126 123M138 113C139 117 138 121 136 124" stroke={coat.marks} strokeWidth="2.4" strokeLinecap="round" />}
        {/* Form shading, the head's shadow on the shoulders, a sheen along the back, and a
            soft shadow over the haunch that gives the back leg a thigh. */}
        <path d={torso} fill={url('torso-form')} />
        <path d={torso} fill={url('neck')} />
        <path d="M113 113.6C122 111.8 134 111.8 143 114.6" stroke="#fff" strokeOpacity=".22" strokeWidth="2.2" strokeLinecap="round" />
        <path d="M140 120C148 120 152 128 150 137C148 142 143 144 138 144C143 137 144 127 140 120Z" fill={coat.shade} fillOpacity=".3" />
      </g>

      {/* Head and collar lean forward a touch while walking. */}
      <g className="cat-front">
      {/* The head turns toward the cursor (outer group) and acts out habits (inner group).
          Ears sit behind it; each is wrapped so CSS can rotate it without overriding
          the mirror transform. */}
      <g className="cat-head-gaze"><g className="cat-head">
      <g className="cat-ear cat-ear-left">{ear(leftEar)}</g>
      <g className="cat-ear cat-ear-right"><g transform="translate(180 0) scale(-1 1)">{ear(rightEar)}</g></g>

      <path d={head} fill={url('head')} />
      {look.coat === 'calico' && <g>
        <path d="M53 92C51 76 60 64 74 61C78 68 77 80 70 88C65 92 58 94 53 92Z" fill={coat.patchA} />
        <path d="M100 60C115 62 127 74 128 88C120 88 110 82 104 74C101 70 100 65 100 60Z" fill={coat.patchB} />
      </g>}
      {tabby && <path d="M83 64L85 72M90 61V71M97 64L95 72M54 96L63 98M53 102L62 102M126 96L117 98M127 102L118 102" stroke={coat.marks} strokeWidth="2.4" strokeLinecap="round" />}
      <path d={head} fill={url('head-form')} />
      <path d={head} {...line} />
      <g className="cat-face">
      {look.coat === 'tuxedo'
        ? <path d="M86 102C87 92 88 84 90 76C92 84 93 92 94 102C100 99 106 101 106 108C106 118 98 124 90 124C82 124 74 118 74 108C74 101 80 99 86 102Z" fill={coat.belly} />
        : look.coat === 'siamese'
          ? <ellipse cx="90" cy="106" rx="27" ry="21" fill={url('mask')} />
          : <path d="M77 109C77 102 83 100 90 104C97 100 103 102 103 109C103 116 97 120 90 120C83 120 77 116 77 109Z" fill={url('muzzle')} />}

      <ellipse cx="63" cy="108" rx="7.5" ry="4.4" fill={url('blush')} />
      <ellipse cx="117" cy="108" rx="7.5" ry="4.4" fill={url('blush')} />
      <g className="cat-blush-strong" stroke="#F0607A" strokeWidth="1.2" strokeLinecap="round">
        <ellipse cx="63" cy="108" rx="8.5" ry="5" fill="#FF7A93" fillOpacity=".4" stroke="none" /><ellipse cx="117" cy="108" rx="8.5" ry="5" fill="#FF7A93" fillOpacity=".4" stroke="none" />
        <path d="M59 110.4L61 105.6M63 110.4L65 105.6M67 110.4L69 105.6M111 110.4L113 105.6M115 110.4L117 105.6M119 110.4L121 105.6" />
      </g>
      <g className="pet-eyes">{eye(74, -1, true)}{eye(106, 1)}</g>
      <path className="cat-eyes-closed" d="M65 95Q74 101 83 95M97 95Q106 101 115 95" stroke={lids} strokeWidth="2.4" strokeLinecap="round" />
      <path className="cat-eyes-happy" d="M66 97Q74 88 82 97M98 97Q106 88 114 97" stroke={lids} strokeWidth="2.4" strokeLinecap="round" />
      <path className="cat-eyes-squeeze" d="M67.5 89.5L79 94.5L67.5 99.5M112.5 89.5L101 94.5L112.5 99.5" stroke={lids} strokeWidth="2.6" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M86.5 106.5C86.5 104.8 93.5 104.8 93.5 106.5C93.5 108.3 91.2 110 90 110C88.8 110 86.5 108.3 86.5 106.5Z" fill={url('nose')} />
      <ellipse cx="88.6" cy="106.2" rx="1.3" ry=".75" fill="#fff" fillOpacity=".55" />
      <path className="pet-mouth" d="M90 110V112.5M85 112.5C86.5 115.5 89 115.5 90 112.5C91 115.5 93.5 115.5 95 112.5" stroke={look.coat === 'siamese' ? '#3A2A22' : coat.outline} strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
      <path className="pet-speaking-mouth" d="M86 113C88 112 92 112 94 113C94 118 92 120 90 120C88 120 86 118 86 113Z" fill="#7A3B4A" />
      <path className="cat-frown" d="M85 116Q90 112 95 116" stroke={look.coat === 'siamese' ? '#3A2A22' : coat.outline} strokeWidth="1.7" strokeLinecap="round" />
      <g className="cat-yawn"><ellipse cx="90" cy="117" rx="5.2" ry="6.8" fill="#7A3B4A" /><ellipse cx="90" cy="120.6" rx="3.2" ry="2.4" fill="#F08A98" /><path d="M86.4 111.6L87.5 114.4L88.6 111ZM93.6 111.6L92.5 114.4L91.4 111Z" fill="#fff" /></g>
      <path className="cat-wink" d="M97 95Q106 100.5 115 95" stroke={lids} strokeWidth="2.6" strokeLinecap="round" />
      <path className="cat-smile" d="M83.5 112C85 119.5 95 119.5 96.5 112C93 113.6 87 113.6 83.5 112Z" fill="#7A3B4A" stroke={look.coat === 'siamese' ? '#3A2A22' : coat.outline} strokeWidth="1.4" strokeLinejoin="round" />
      <path className="cat-tongue" d="M87.4 113C87.4 119.4 92.6 119.4 92.6 113Z" fill="#F08A98" stroke="#C9606F" strokeWidth=".8" />
      <path d="M58 106Q50 103.4 42 103M58 110Q50 110 41 111.4M59 114Q52 115.4 45 118.4M122 106Q130 103.4 138 103M122 110Q130 110 139 111.4M121 114Q128 115.4 135 118.4" stroke={coat.whisker} strokeWidth="1" strokeLinecap="round" strokeOpacity=".72" />
      </g>

      {look.accessory === 'bow' && <g transform="rotate(18 117 58) translate(117 58) scale(1.25) translate(-117 -58)" stroke="#000" strokeOpacity=".2" strokeWidth="1.1" strokeLinejoin="round">
        <path d="M117 58C111 51 105 53 106 59C107 65 113 63 117 58Z" fill={accent} />
        <path d="M117 58C123 51 129 53 128 59C127 65 121 63 117 58Z" fill={accent} />
        <path d="M113.5 56.4C111 54.6 108.6 55 108.2 57.4M120.5 56.4C123 54.6 125.4 55 125.8 57.4" stroke="#fff" strokeOpacity=".4" strokeWidth="1" strokeLinecap="round" />
        <path d="M115 60L112 68M119 60L122 68" stroke={accent} strokeOpacity="1" strokeWidth="2.4" strokeLinecap="round" />
        <circle cx="117" cy="58" r="3.2" fill={mix(accent, '#1D1720', .12)} />
      </g>}
      {look.accessory === 'hat' && <g transform="rotate(-10 90 60) translate(90 61) scale(1.35) translate(-90 -61)" stroke="#fff" strokeOpacity=".35" strokeWidth=".9">
        <path d="M80 60V44C80 42.5 81.5 41 83 41H97C98.5 41 100 42.5 100 44V60Z" fill="#3A3245" />
        <rect x="80" y="53" width="20" height="4" fill={accent} stroke="none" />
        <ellipse cx="90" cy="60.5" rx="16" ry="3.6" fill="#3A3245" />
        <path d="M83.5 45V51" stroke="#fff" strokeOpacity=".25" strokeWidth="1.6" strokeLinecap="round" />
      </g>}
      {look.accessory === 'flowers' && <g>
        <path d="M71 65L73 63M83 59L85 59M96 59L98 59M108 63L110 65" stroke="#7FB069" strokeWidth="3" strokeLinecap="round" />
        {flowers.map(([x, y], index) => <g key={index}>
          {[0, 72, 144, 216, 288].map(angle => <circle key={angle} cx={x + 2.9 * Math.cos(angle * Math.PI / 180)} cy={y + 2.9 * Math.sin(angle * Math.PI / 180)} r="2.6" fill={[accent, '#FF9FB5', '#FFFFFF'][index % 3]} stroke="#000" strokeOpacity=".08" strokeWidth=".6" />)}
          <circle cx={x} cy={y} r="1.8" fill="#FFD45C" />
        </g>)}
      </g>}
      {/* Feelings that sit on the head: a cross little vein, a nervous drop, a sigh's puff. */}
      <path className="cat-emote cat-emote-vein" d="M107 64.5Q110.5 64.5 110.5 61M113.5 61Q113.5 64.5 117 64.5M117 67.5Q113.5 67.5 113.5 71M110.5 71Q110.5 67.5 107 67.5" stroke="#E5484D" strokeWidth="2" strokeLinecap="round" />
      <g className="cat-emote cat-emote-sweat"><path d="M133 68C135.6 71.6 137.4 74.4 137.4 76.6A4.4 4.4 0 0 1 128.6 76.6C128.6 74.4 130.4 71.6 133 68Z" fill="#9AD8F5" stroke="#4A9CC9" strokeWidth=".9" /><ellipse cx="131.4" cy="76" rx="1" ry="1.6" fill="#fff" fillOpacity=".8" /></g>
      <g className="cat-emote cat-emote-puff" fill="#fff" fillOpacity=".92" stroke="#1D1720" strokeOpacity=".14" strokeWidth=".8"><circle cx="54" cy="123" r="3.6" /><circle cx="49.4" cy="121" r="2.6" /><circle cx="58.4" cy="120.4" r="2.2" /></g>
      </g></g>

      {/* A fluffy ruff spills over the chin; a collar or bandana covers it. It belongs to
          the seated chest, so it fades out with the seated body. */}
      <g className="cat-ruff"><path d="M74 125.6C80 127.8 85 128.4 90 128.4C95 128.4 100 127.8 106 125.6L104 128.6Q102.6 130.6 101 132.2Q99.6 131.6 97.4 130.4Q95.6 131.8 94.6 134.2Q93 133.4 90.6 131.4Q88.6 132.4 87.6 134.8Q85.8 133.6 83.2 130.4Q81.4 130.8 80.4 132.6Q78.4 131.4 76 128.6Z" fill={url('ruff')} />
      <path d="M76 128.6Q78.4 131.4 80.4 132.6Q81.4 130.8 83.2 130.4Q85.8 133.6 87.6 134.8Q88.6 132.4 90.6 131.4Q93 133.4 94.6 134.2Q95.6 131.8 97.4 130.4Q99.6 131.6 101 132.2Q102.6 130.6 104 128.6" stroke={coat.outline} strokeOpacity=".4" strokeWidth="1.2" strokeLinecap="round" strokeLinejoin="round" /></g>

      {look.accessory === 'bell' && <g>
        <path d="M67 121Q90 133 113 121" stroke={coat.outline} strokeOpacity=".7" strokeWidth="7" strokeLinecap="round" />
        <path d="M67 121Q90 133 113 121" stroke={accent} strokeWidth="4.6" strokeLinecap="round" />
        <path d="M70 120.6Q90 130.6 110 120.6" stroke="#fff" strokeOpacity=".3" strokeWidth="1.2" strokeLinecap="round" />
        <circle cx="90" cy="130.5" r="5" fill={url('bell')} stroke="#B5891F" strokeWidth="1.2" />
        <path d="M87.5 132H92.5" stroke="#8A6614" strokeWidth="1" strokeLinecap="round" />
        <circle cx="88.4" cy="128.8" r="1.2" fill="#fff" fillOpacity=".8" />
      </g>}
      {look.accessory === 'bandana' && <g>
        <path d="M66 120Q90 132 114 120L108 128Q99 146 90 149Q81 146 72 128Z" fill={accent} stroke="#000" strokeOpacity=".18" strokeWidth="1.2" strokeLinejoin="round" />
        <path d="M72 128Q90 136 108 128" stroke="#000" strokeOpacity=".1" strokeWidth="1.2" strokeLinecap="round" />
        <circle cx="84" cy="133" r="1.4" fill="#fff" fillOpacity=".8" /><circle cx="96" cy="133" r="1.4" fill="#fff" fillOpacity=".8" /><circle cx="90" cy="140" r="1.4" fill="#fff" fillOpacity=".8" />
      </g>}
      </g>
      {/* A copy of the right paw that comes up to the face to groom, to wave, or to hide a
          bashful smile. */}
      <g className="cat-groom-paw">
        <path d="M93 156C93 151 97 149 102 149C107 149 110 151 110 156C110 161 107 163 102 163C97 163 93 161 93 156Z" fill={url('paw')} {...line} />
        <path d="M99 151V154M105 151V154" stroke={coat.outline} strokeWidth="1.2" strokeLinecap="round" strokeOpacity=".55" />
      </g>
    </g>
    {/* A ball of wool in the accent colour, beside the front paw. The cat bats it while
        playing (the `bat` habit); a loose strand runs back to the paw. */}
    <g className="cat-yarn">
      <ellipse className="cat-yarn-shadow" cx="38" cy="166" rx="12" ry="2.8" fill={url('shadow')} />
      <path className="cat-yarn-strand" d="M47 162C54 168 61 167 66 162C68 160 70 159 72 160" stroke={accent} strokeWidth="1.6" strokeLinecap="round" />
      <g className="cat-yarn-ball">
        <circle cx="38" cy="155" r="11" fill={url('yarn')} />
        <g stroke="#000" strokeOpacity=".2" strokeWidth="1.2" strokeLinecap="round">
          <path d="M28.6 150C33 146 42 145.6 47.6 150.6" />
          <path d="M27.4 156C33.6 151.4 42.6 151.6 48.8 157.6" />
          <path d="M30.6 163C35 158.8 42 158.6 46.4 162.4" />
          <path d="M33.6 145C30.4 151 31 159 35.8 165.4" />
          <path d="M42.6 144.8C45.8 151 45.4 159.4 40.6 165.6" />
        </g>
        <path d="M31.4 149.4C33 147.4 35.4 146.2 37.8 146" stroke="#fff" strokeOpacity=".55" strokeWidth="1.6" strokeLinecap="round" />
        <circle cx="38" cy="155" r="11" stroke="#000" strokeOpacity=".22" strokeWidth="1.2" />
      </g>
    </g>
    <g className="cat-hearts" fill={url('heart')}>{[[124, 68], [137, 54], [113, 50]].map(([x, y], index) => <g key={index} transform={`translate(${x} ${y})`}><g className="cat-heart" style={{ animationDelay: `${index * .28}s` }}><path d="M0 3C-1.5 0-6-.5-6 3C-6 6.5-1.5 8.5 0 10C1.5 8.5 6 6.5 6 3C6-.5 1.5 0 0 3Z" /><ellipse cx="-3.2" cy="2.6" rx="1.3" ry=".9" fill="#fff" fillOpacity=".6" /></g></g>)}</g>
    <g className="cat-ask"><path d="M146 18H166C171 18 174 21 174 26V34C174 39 171 42 166 42H155L149 48L150 42H146C141 42 138 39 138 34V26C138 21 141 18 146 18Z" fill="#FFFDF8" stroke={coat.outline} strokeOpacity=".45" strokeWidth="1.2" strokeLinejoin="round" /><path className="cat-ask-heart" d="M156 27C154.4 24 149.5 24.4 149.5 28.4C149.5 32 154 34.6 156 36.4C158 34.6 162.5 32 162.5 28.4C162.5 24.4 157.6 24 156 27Z" fill={url('heart')} /></g>
    <g className="cat-zzz" fill="#8C7FA6" stroke="#fff" strokeOpacity=".7" strokeWidth="2.4" strokeLinejoin="round" paintOrder="stroke" fontFamily="-apple-system, system-ui, sans-serif" fontWeight="800"><text x="128" y="60" fontSize="12">z</text><text x="137" y="48" fontSize="9">z</text><text x="144" y="38" fontSize="7">z</text></g>
    {/* Feelings that pop up beside the head: surprise, a question, delight. */}
    <g className="cat-emote cat-emote-alert" fill="#FF9F43" stroke="#fff" strokeOpacity=".8" strokeWidth="2" paintOrder="stroke">
      <path d="M139.6 28.5C139.6 26.6 144.4 26.6 144.4 28.5L143.6 41C143.5 42.4 140.5 42.4 140.4 41Z" /><circle cx="142" cy="46.6" r="2.7" />
      <path d="M133.4 29.6L136 32.4M150.6 29.6L148 32.4M131.6 38.6H135" stroke="#FF9F43" strokeOpacity="1" strokeWidth="1.6" strokeLinecap="round" />
    </g>
    <g className="cat-emote cat-emote-question" fill="#8C7FA6" stroke="#fff" strokeOpacity=".75" strokeWidth="2.4" strokeLinejoin="round" paintOrder="stroke" fontFamily="-apple-system, system-ui, sans-serif" fontWeight="800"><text x="135" y="48" fontSize="22">?</text></g>
    <g className="cat-emote cat-emote-sparkle" fill="#FFD45C" stroke="#fff" strokeOpacity=".7" strokeWidth="1" strokeLinejoin="round" paintOrder="stroke">{[[140, 50, 5.5], [151, 36, 3.6], [38, 56, 4.4]].map(([x, y, r], index) => <path key={index} className="cat-sparkle" style={{ animationDelay: `${index * .22}s` }} d={star(x, y, r)} />)}</g>
  </svg>;
}
