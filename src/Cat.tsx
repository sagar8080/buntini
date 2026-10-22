import type { CatCoat, PetLook } from './types';
import { eyeColors } from './petOptions';

export interface Coat {
  base: string; shade: string; outline: string; belly: string; ear: string; nose: string; whisker: string;
  marks?: string; patchA?: string; patchB?: string; point?: string; blush: number;
}

export const palettes: Record<CatCoat, Coat> = {
  ginger: { base: '#F6AA5E', shade: '#E2843D', outline: '#9A5126', belly: '#FFF1DC', ear: '#F7B3A6', nose: '#F08A98', whisker: '#9A5126', marks: '#D46F2D', blush: .38 },
  gray: { base: '#A9B1BE', shade: '#8A93A2', outline: '#4E5563', belly: '#F2F3F6', ear: '#F2B8B5', nose: '#E58C9A', whisker: '#4E5563', marks: '#6E7787', blush: .38 },
  black: { base: '#3E3B45', shade: '#2B2930', outline: '#141217', belly: '#4B4753', ear: '#C98A93', nose: '#7A505D', whisker: '#CFCAD6', blush: .24 },
  white: { base: '#FFFDF8', shade: '#ECE5DB', outline: '#A99E92', belly: '#FFFFFF', ear: '#F7B9B6', nose: '#F08A98', whisker: '#B3A79B', blush: .42 },
  tuxedo: { base: '#37343D', shade: '#27252C', outline: '#141217', belly: '#FFFFFF', ear: '#C98A93', nose: '#F09AA6', whisker: '#D8D3DE', blush: .3 },
  calico: { base: '#FFFBF4', shade: '#EDE4D8', outline: '#9C8D7E', belly: '#FFFFFF', ear: '#F7B9B6', nose: '#F08A98', whisker: '#A8998A', patchA: '#EF9B4E', patchB: '#433D44', blush: .4 },
  siamese: { base: '#F6ECDC', shade: '#E4D3BB', outline: '#7A5A45', belly: '#FBF5EC', ear: '#8A6352', nose: '#5E4032', whisker: '#7A5A45', point: '#5E4032', blush: .3 },
};

const flowers: [number, number][] = [[66, 69], [77, 62], [90, 59], [103, 62], [114, 69]];

/* Seated, round-headed kitten. Coat, eyes and accessory come from the saved look;
   state reactions are CSS-driven so the SVG stays static and cheap to render. */
export type CatPose = 'sit' | 'walk' | 'leap';

export default function Cat({ id, look, pose = 'sit' }: { id: string; look: PetLook; pose?: CatPose }) {
  const idPrefix = id;
  const coat = palettes[look.coat];
  const accent = look.accent;
  const tabby = look.coat === 'ginger' || look.coat === 'gray';
  const leftIris = look.eyes === 'odd' ? eyeColors.blue : eyeColors[look.eyes];
  const rightIris = look.eyes === 'odd' ? eyeColors.amber : eyeColors[look.eyes];
  const tail = coat.point ?? coat.patchB ?? coat.base;
  const paws = look.coat === 'tuxedo' ? coat.belly : coat.point ?? coat.base;
  const leftEar = coat.point ?? coat.patchA ?? coat.base;
  const rightEar = coat.point ?? coat.patchB ?? coat.base;
  const line = { stroke: coat.outline, strokeWidth: 1.8, strokeLinejoin: 'round' as const };

  // A walking leg is jointed: the upper leg turns at the hip (x, 131), the lower leg at
  // the knee and the paw at the ankle, so a step can plant, push, fold and reach.
  // Each joint sits at its group's origin; CSS animates the three groups (styles.css).
  // The leg is drawn twice, outline underneath and fill on top, so the joints merge into
  // one smooth silhouette instead of showing seams.
  const leg = (x: number, id: 'nf' | 'nb' | 'ff' | 'fb', end: 'front' | 'back', far: boolean) => {
    const fill = `url(#${idPrefix}-leg${far ? '-far' : ''})`;
    const pawFill = far ? coat.shade : paws;
    const layer = (outline: boolean) => {
      const paint = (color: string) => outline ? { fill: coat.outline, stroke: coat.outline, strokeWidth: 3.6, strokeLinejoin: 'round' as const } : { fill: color };
      return <g className={`walk-leg leg-${id} ${end}`}>
        <path d="M-6.6 -4C-6.6 -7 6.6 -7 6.6 -4L4.9 13.2C4.6 16.2 -4.6 16.2 -4.9 13.2Z" {...paint(fill)} />
        <g transform="translate(0 14)"><g className="walk-shin">
          <path d="M-4.6 -2.6C-4.6 -4.6 4.6 -4.6 4.6 -2.6L3.6 12.2C3.5 14.4 -3.5 14.4 -3.6 12.2Z" {...paint(fill)} />
          <g transform="translate(0 13)"><g className="walk-paw">
            <path d="M-7.4 1.8C-7.4 -1.2 -4.6 -3 -1.2 -3C2.6 -3 4.8 -1.2 4.8 1.8C4.8 4.3 2.4 5.4 -1.4 5.4C-5.1 5.4 -7.4 4.3 -7.4 1.8Z" {...paint(pawFill)} />
            {!outline && !far && <path d="M-4.6 2.6V4.6M-1.4 2.8V5" stroke={coat.outline} strokeWidth="1" strokeLinecap="round" strokeOpacity=".5" />}
          </g></g>
        </g></g>
      </g>;
    };
    return <g transform={`translate(${x} 131)`} key={id}>{layer(true)}{layer(false)}</g>;
  };
  const ear = (fill: string) => <>
    <path d="M57 82C53 68 52 54 55 45C56 41 60 40 63 42C71 48 78 55 84 63Z" fill={fill} {...line} />
    <path d="M61 73C59 63 59 55 60 49C66 53 71 58 76 63Z" fill={coat.ear} />
  </>;
  const eye = (cx: number, iris: string) => <>
    <ellipse cx={cx} cy="94" rx="9" ry="10" fill={iris} stroke="#2A2230" strokeWidth="1.4" />
    <g className="cat-look"><g className="cat-pupil">
      <ellipse cx={cx} cy="94.5" rx="5.3" ry="7.2" fill="#221B27" />
      <circle cx={cx - 3} cy="89.6" r="2.8" fill="#fff" />
      <circle cx={cx + 3.4} cy="98.6" r="1.3" fill="#fff" fillOpacity=".9" />
    </g></g>
  </>;

  return <svg viewBox="0 0 180 180" fill="none" xmlns="http://www.w3.org/2000/svg" className={`cat cat--${pose}`}>
    <defs>
      <linearGradient id={`${id}-coat`} x1="90" y1="56" x2="90" y2="164" gradientUnits="userSpaceOnUse"><stop stopColor={coat.base} /><stop offset=".62" stopColor={coat.base} /><stop offset="1" stopColor={coat.shade} /></linearGradient>
      <linearGradient id={`${id}-leg`} x1="0" y1="0" x2="1" y2="0"><stop stopColor={coat.point ?? coat.base} /><stop offset=".65" stopColor={coat.point ?? coat.base} /><stop offset="1" stopColor={coat.shade} /></linearGradient>
      <linearGradient id={`${id}-leg-far`} x1="0" y1="0" x2="1" y2="0"><stop stopColor={coat.shade} /><stop offset="1" stopColor={coat.outline} stopOpacity=".3" /></linearGradient>
      <radialGradient id={`${id}-mask`} cx="90" cy="106" r="27" gradientUnits="userSpaceOnUse"><stop stopColor={coat.point} stopOpacity=".92" /><stop offset=".55" stopColor={coat.point} stopOpacity=".55" /><stop offset="1" stopColor={coat.point} stopOpacity="0" /></radialGradient>
    </defs>
    <ellipse className="pet-shadow cat-sit-shadow" cx="90" cy="165" rx="38" ry="5" fill="#1D1720" fillOpacity=".18" />
    <ellipse className="pet-shadow cat-walk-shadow" cx="114" cy="165" rx="46" ry="5" fill="#1D1720" fillOpacity=".18" />
    <g className="pet-body">
      {/* Seated body. While walking it fades out and the walking body below fades in;
          the head never changes, so switching poses reads as the cat getting up. */}
      <g className="cat-sit">
      <g className="cat-tail">
        <path d="M110 154C132 160 150 150 151 128C152 116 146 106 138 104" stroke={coat.outline} strokeWidth="13.5" strokeLinecap="round" />
        <path d="M110 154C132 160 150 150 151 128C152 116 146 106 138 104" stroke={tail} strokeWidth="10" strokeLinecap="round" />
        {tabby && <path d="M110 154C132 160 150 150 151 128C152 116 146 106 138 104" stroke={coat.marks} strokeWidth="10" strokeDasharray="4 7" strokeDashoffset="-8" />}
      </g>

      <path d="M66 122C57 134 56 150 63 158C69 164 111 164 117 158C124 150 123 134 114 122Z" fill={`url(#${id}-coat)`} {...line} />
      {look.coat === 'calico' && <g>
        <path d="M66 124C60 134 59 146 62 154C68 152 72 144 72 134C71 129 69 126 66 124Z" fill={coat.patchA} />
        <path d="M114 124C120 134 121 144 118 152C112 150 108 142 108 134C109 129 111 126 114 124Z" fill={coat.patchB} />
      </g>}
      <path d="M79 126C74 136 74 150 79 158H101C106 150 106 136 101 126C95 130 85 130 79 126Z" fill={coat.belly} />
      {tabby && <path d="M63 134C67 136 70 135 72 132M61 143C66 145 69 144 71 141M62 152C66 153 69 152 70 150M117 134C113 136 110 135 108 132M119 143C114 145 111 144 109 141M118 152C114 153 111 152 110 150" stroke={coat.marks} strokeWidth="2.4" strokeLinecap="round" />}
      <g className="cat-paw cat-paw-left">
        <path d="M70 156C70 151 74 149 79 149C84 149 87 151 87 156C87 161 84 163 79 163C74 163 70 161 70 156Z" fill={paws} {...line} />
        <path d="M76 158V161M82 158V161" stroke={coat.outline} strokeWidth="1.2" strokeLinecap="round" strokeOpacity=".55" />
      </g>
      <g className="cat-paw cat-paw-right">
        <path d="M93 156C93 151 97 149 102 149C107 149 110 151 110 156C110 161 107 163 102 163C97 163 93 161 93 156Z" fill={paws} {...line} />
        <path d="M99 158V161M105 158V161" stroke={coat.outline} strokeWidth="1.2" strokeLinecap="round" strokeOpacity=".55" />
      </g>
      </g>

      {/* Walking body, side-on behind the head: far legs, tail, torso, near legs. */}
      <g className="cat-walkbody">
        {leg(111, 'ff', 'front', true)}
        {leg(146, 'fb', 'back', true)}
        <g className="walk-tail">
          <path d="M150 122C164 118 170 104 166 90C164 83 159 81 157 85" stroke={coat.outline} strokeWidth="13" strokeLinecap="round" />
          <path d="M150 122C164 118 170 104 166 90C164 83 159 81 157 85" stroke={tail} strokeWidth="9.5" strokeLinecap="round" />
          {tabby && <path d="M150 122C164 118 170 104 166 90C164 83 159 81 157 85" stroke={coat.marks} strokeWidth="9.5" strokeDasharray="4 6" />}
        </g>
        {leg(101, 'nf', 'front', false)}
        {leg(136, 'nb', 'back', false)}
        <path d="M96 128C96 116 106 110 120 110H134C146 110 153 117 153 128C153 139 146 145 134 145H112C102 145 96 139 96 128Z" fill={`url(#${id}-coat)`} {...line} />
        {look.coat === 'calico' && <g>
          <path d="M116 111C124 109 132 111 135 116C129 121 119 121 113 117C112 114 113 112 116 111Z" fill={coat.patchA} />
          <path d="M140 112C148 114 153 120 153 128C147 128 141 124 138 118Z" fill={coat.patchB} />
        </g>}
        <path d="M104 138C112 145 132 146 145 140C141 145 137 145 133 145H112C108 145 105 142 104 138Z" fill={coat.belly} />
        {look.coat === 'tuxedo' && <path d="M96 128C96 138 102 144 110 145C113 137 112 124 105 117C100 119 96 123 96 128Z" fill={coat.belly} />}
        {tabby && <path d="M118 111C119 116 118 120 116 123M128 111C129 116 128 120 126 123M138 113C139 117 138 121 136 124" stroke={coat.marks} strokeWidth="2.4" strokeLinecap="round" />}
        {/* Shoulder and haunch: soft shading where the legs join, so they read as part of a body. */}
        <path d="M140 120C148 120 152 128 150 137C148 142 143 144 138 144C143 137 144 127 140 120Z" fill={coat.shade} fillOpacity=".45" />
        <path d="M129 121C137 117 146 121 148 130" stroke={coat.shade} strokeWidth="1.6" strokeLinecap="round" strokeOpacity=".7" />
        <path d="M107 120C101 123 99 130 101 137" stroke={coat.shade} strokeWidth="1.6" strokeLinecap="round" strokeOpacity=".7" />
      </g>

      {/* Head and collar lean forward a touch while walking. */}
      <g className="cat-front">
      {/* The head turns toward the cursor (outer group) and acts out habits (inner group).
          Ears sit behind it; each is wrapped so CSS can rotate it without overriding
          the mirror transform. */}
      <g className="cat-head-gaze"><g className="cat-head">
      <g className="cat-ear cat-ear-left">{ear(leftEar)}</g>
      <g className="cat-ear cat-ear-right"><g transform="translate(180 0) scale(-1 1)">{ear(rightEar)}</g></g>

      <path d="M52 98C49 76 66 58 90 58C114 58 131 76 128 98L134 103L127 105L131 111L123 110C117 122 105 128 90 128C75 128 63 122 57 110L49 111L53 105L46 103Z" fill={`url(#${id}-coat)`} {...line} />
      {look.coat === 'calico' && <g>
        <path d="M53 92C51 76 60 64 74 61C78 68 77 80 70 88C65 92 58 94 53 92Z" fill={coat.patchA} />
        <path d="M100 60C115 62 127 74 128 88C120 88 110 82 104 74C101 70 100 65 100 60Z" fill={coat.patchB} />
      </g>}
      {tabby && <path d="M83 64L85 72M90 61V71M97 64L95 72M54 96L63 98M53 102L62 102M126 96L117 98M127 102L118 102" stroke={coat.marks} strokeWidth="2.4" strokeLinecap="round" />}
      {look.coat === 'tuxedo'
        ? <path d="M86 102C87 92 88 84 90 76C92 84 93 92 94 102C100 99 106 101 106 108C106 118 98 124 90 124C82 124 74 118 74 108C74 101 80 99 86 102Z" fill={coat.belly} />
        : look.coat === 'siamese'
          ? <ellipse cx="90" cy="106" rx="27" ry="22" fill={`url(#${id}-mask)`} />
          : <path d="M77 109C77 102 83 100 90 104C97 100 103 102 103 109C103 116 97 120 90 120C83 120 77 116 77 109Z" fill={coat.belly} />}
      {look.coat !== 'white' && look.coat !== 'calico' && <path d="M70 66C76 62 82 61 87 61" stroke="#fff" strokeOpacity=".3" strokeWidth="2.5" strokeLinecap="round" />}

      <ellipse cx="63" cy="108" rx="6" ry="3.4" fill="#FF8DA1" fillOpacity={coat.blush} />
      <ellipse cx="117" cy="108" rx="6" ry="3.4" fill="#FF8DA1" fillOpacity={coat.blush} />
      <g className="pet-eyes">{eye(74, leftIris)}{eye(106, rightIris)}</g>
      <path className="cat-eyes-closed" d="M65 95Q74 101 83 95M97 95Q106 101 115 95" stroke={look.coat === 'black' || look.coat === 'tuxedo' ? '#F4E9DA' : look.coat === 'siamese' ? '#2E1F18' : coat.outline} strokeWidth="2.4" strokeLinecap="round" />
      <path className="cat-eyes-happy" d="M66 97Q74 88 82 97M98 97Q106 88 114 97" stroke={look.coat === 'black' || look.coat === 'tuxedo' ? '#F4E9DA' : look.coat === 'siamese' ? '#2E1F18' : coat.outline} strokeWidth="2.4" strokeLinecap="round" />
      <path d="M86.5 106.5C86.5 104.8 93.5 104.8 93.5 106.5C93.5 108.3 91.2 110 90 110C88.8 110 86.5 108.3 86.5 106.5Z" fill={coat.nose} />
      <path className="pet-mouth" d="M90 110V112.5M85 112.5C86.5 115.5 89 115.5 90 112.5C91 115.5 93.5 115.5 95 112.5" stroke={look.coat === 'siamese' ? '#3A2A22' : coat.outline} strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
      <path className="pet-speaking-mouth" d="M86 113C88 112 92 112 94 113C94 118 92 120 90 120C88 120 86 118 86 113Z" fill="#7A3B4A" />
      <path className="cat-frown" d="M85 116Q90 112 95 116" stroke={look.coat === 'siamese' ? '#3A2A22' : coat.outline} strokeWidth="1.7" strokeLinecap="round" />
      <g className="cat-yawn"><ellipse cx="90" cy="117" rx="5.2" ry="6.8" fill="#7A3B4A" /><ellipse cx="90" cy="120.6" rx="3.2" ry="2.4" fill="#F08A98" /></g>
      <path d="M58 106L42 103M58 110L41 111M59 114L45 118M122 106L138 103M122 110L139 111M121 114L135 118" stroke={coat.whisker} strokeWidth="1.1" strokeLinecap="round" strokeOpacity=".75" />

      {look.accessory === 'bow' && <g transform="rotate(18 117 58) translate(117 58) scale(1.25) translate(-117 -58)" stroke="#000" strokeOpacity=".2" strokeWidth="1.1" strokeLinejoin="round">
        <path d="M117 58C111 51 105 53 106 59C107 65 113 63 117 58Z" fill={accent} />
        <path d="M117 58C123 51 129 53 128 59C127 65 121 63 117 58Z" fill={accent} />
        <path d="M115 60L112 68M119 60L122 68" stroke={accent} strokeOpacity="1" strokeWidth="2.4" strokeLinecap="round" />
        <circle cx="117" cy="58" r="3.2" fill={accent} />
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
      </g></g>

      {look.accessory === 'bell' && <g>
        <path d="M67 121Q90 133 113 121" stroke={coat.outline} strokeOpacity=".7" strokeWidth="7" strokeLinecap="round" />
        <path d="M67 121Q90 133 113 121" stroke={accent} strokeWidth="4.6" strokeLinecap="round" />
        <circle cx="90" cy="130.5" r="5" fill="#F6C945" stroke="#B5891F" strokeWidth="1.2" />
        <path d="M87.5 132H92.5" stroke="#8A6614" strokeWidth="1" strokeLinecap="round" />
        <circle cx="88.4" cy="128.8" r="1.2" fill="#fff" fillOpacity=".75" />
      </g>}
      {look.accessory === 'bandana' && <g>
        <path d="M66 120Q90 132 114 120L108 128Q99 146 90 149Q81 146 72 128Z" fill={accent} stroke="#000" strokeOpacity=".18" strokeWidth="1.2" strokeLinejoin="round" />
        <circle cx="84" cy="133" r="1.4" fill="#fff" fillOpacity=".8" /><circle cx="96" cy="133" r="1.4" fill="#fff" fillOpacity=".8" /><circle cx="90" cy="140" r="1.4" fill="#fff" fillOpacity=".8" />
      </g>}
      </g>
      {/* A copy of the right paw drawn in front of the face, shown only while grooming. */}
      <g className="cat-groom-paw">
        <path d="M93 156C93 151 97 149 102 149C107 149 110 151 110 156C110 161 107 163 102 163C97 163 93 161 93 156Z" fill={paws} {...line} />
        <path d="M99 151V154M105 151V154" stroke={coat.outline} strokeWidth="1.2" strokeLinecap="round" strokeOpacity=".55" />
      </g>
    </g>
    <g className="cat-hearts" fill="#FF7A93">{[[124, 68], [137, 54], [113, 50]].map(([x, y], index) => <g key={index} transform={`translate(${x} ${y})`}><path className="cat-heart" style={{ animationDelay: `${index * .28}s` }} d="M0 3C-1.5 0-6-.5-6 3C-6 6.5-1.5 8.5 0 10C1.5 8.5 6 6.5 6 3C6-.5 1.5 0 0 3Z" /></g>)}</g>
    <g className="cat-zzz" fill="#8C7FA6" fontFamily="Manrope, sans-serif" fontWeight="800"><text x="128" y="60" fontSize="12">z</text><text x="137" y="48" fontSize="9">z</text><text x="144" y="38" fontSize="7">z</text></g>
  </svg>;
}
