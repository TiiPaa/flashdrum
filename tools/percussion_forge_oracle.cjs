// Capture deterministic references from the supplied Percussion Forge app.
// Development tool only: Node is not required by the plugin.
// Usage: node tools/percussion_forge_oracle.cjs [path/to/dist/app.js]
'use strict';
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const crypto = require('node:crypto');
const root = path.resolve(__dirname, '..');
const sourcePath = process.argv[2] || path.join(root, 'resources/percussion-forge/dist/app.js');
const source = fs.readFileSync(sourcePath, 'utf8');
const prefix = source.slice(0, source.indexOf('function voiceUI()'));
if (!prefix || !prefix.includes('function synth(')) throw new Error('Unexpected source');
const sandbox = {};
vm.createContext(sandbox);
const instrumented = prefix.replace('const scale=.92/Math.max(.92,peak);', 'globalThis.wideGain=.92/Math.max(.92,peak);const scale=globalThis.wideGain;');
vm.runInContext(instrumented + '\nglobalThis.reference = {voices, synth, widenNoise};', sandbox);
const {voices, synth, widenNoise} = sandbox.reference;
const rawSandbox = {};
vm.createContext(rawSandbox);
const rawPrefix = prefix.replace('const scale=.88/Math.max(1,peak);', 'const scale=1;');
if (rawPrefix === prefix) throw new Error('Normalization anchor changed');
vm.runInContext(rawPrefix + '\nglobalThis.rawSynth = synth;', rawSandbox);
const slotIndices = [0, 1, 2, 3, 5, 6, 7, 8, 9, 10];
const fixtures = [];
function stats(data) {
  let peak = 0, power = 0, sum = 0;
  for (const v of data) { peak = Math.max(peak, Math.abs(v)); power += v*v; sum += v; }
  return {peak, rms: Math.sqrt(power/data.length), mean: sum/data.length};
}
for (let kind = 0; kind < voices.length; ++kind) {
  const v = voices[kind];
  const defaults = Object.fromEntries(v.params.map(p => [p[0],p[4]]));
  const cases = [['default',defaults], ...Object.entries(v.presets).map(([n,p]) => [n,{...defaults,...p}])];
  for (const p of v.params) for (const end of [2,3]) cases.push([`${p[0]}-${end===2?'min':'max'}`,{...defaults,[p[0]]:p[end]}]);
  for (const [name,params] of cases) {
    const data = synth(v.id,params,1,42,0);
    const raw = rawSandbox.rawSynth(v.id,params,1,42,0);
    const extra = v.params.filter(p => !['pitch','tone','base','decay'].includes(p[0]));
    fixtures.push({kind,name,frequency:params.pitch??params.tone??params.base,decay:params.decay/1000,
      special:extra.map((p,i)=>[slotIndices[i],params[p[0]]]),
      samples:data.length,...stats(data),rawPeak:stats(raw).peak,prefix:Array.from(data.slice(0,1024))});
  }
  for (const velocity of [.2,.65]) {
    const data=synth(v.id,defaults,velocity,42,0);
    const raw=rawSandbox.rawSynth(v.id,defaults,velocity,42,0);
    fixtures.push({kind,name:`velocity-${velocity}`,frequency:defaults.pitch??defaults.tone??defaults.base,
      decay:defaults.decay/1000,special:v.params.filter(p=>!['pitch','tone','base','decay'].includes(p[0])).map((p,i)=>[slotIndices[i],defaults[p[0]]]),
      velocity,samples:data.length,...stats(data),rawPeak:stats(raw).peak,prefix:Array.from(data.slice(0,1024))});
  }
  if (!['zap','subfm'].includes(v.id)) {
    const rawL=synth(v.id,defaults,1,42,-1),rawR=synth(v.id,defaults,1,7961,1);
    const [l,r]=widenNoise(rawL,rawR,v.id);
    const leftPeak=stats(rawSandbox.rawSynth(v.id,defaults,1,42,-1)).peak;
    const rightPeak=stats(rawSandbox.rawSynth(v.id,defaults,1,7961,1)).peak;
    fixtures.push({kind,name:'stereo',frequency:defaults.pitch??defaults.tone??defaults.base,
      decay:defaults.decay/1000,special:v.params.filter(p=>!['pitch','tone','base','decay'].includes(p[0])).map((p,i)=>[slotIndices[i],defaults[p[0]]]),
      samples:l.length,left:stats(l),right:stats(r),rawLeftPeak:leftPeak,rawRightPeak:rightPeak,wideGain:sandbox.wideGain,
      prefix:Array.from(l.slice(0,1024)),rightPrefix:Array.from(r.slice(0,1024))});
  }
}
const result={sourceSha256:crypto.createHash('sha256').update(source).digest('hex'),sampleRate:48000,seed:42,fixtures};
const out=path.join(root,'drum-pattern-vst/tests/fixtures/percussion-forge.json');
fs.mkdirSync(path.dirname(out),{recursive:true});
fs.writeFileSync(out,JSON.stringify(result));
console.log(`${fixtures.length} reference cases -> ${out}`);
for(let kind=0;kind<9;kind++) {
  const rows=fixtures.filter(f=>f.kind===kind&&!f.left);
  console.log(`${voices[kind].name}: ${rows.length} mono cases, maximum peak ${Math.max(...rows.map(r=>r.peak)).toFixed(6)}`);
}
