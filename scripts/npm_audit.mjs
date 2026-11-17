// The npm audit for CI and releases: every package, the build toolchain included. Fails on
// any high or critical advisory except one listed in npm-audit-exceptions.json, which counts
// only while its advisory, package and affected range are exactly as reviewed and its date to
// look again hasn't passed. A fix being published changes the range, so the exception lapses
// and the audit fails until the fix is taken. An audit that can't run fails too.
import { spawnSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const exceptions = JSON.parse(readFileSync(join(here, 'npm-audit-exceptions.json'), 'utf8'));
const today = new Date().toISOString().slice(0, 10);

const run = spawnSync('npm', ['audit', '--json'], { cwd: join(here, '..'), encoding: 'utf8', maxBuffer: 64 << 20 });
let report;
try {
  report = JSON.parse(run.stdout);
} catch {
  console.error(`npm audit gave no report (exit ${run.status}).\n${run.stderr}`);
  process.exit(1);
}
if (report.error || !report.vulnerabilities || !report.metadata) {
  console.error(`npm audit didn't finish: ${JSON.stringify(report.error ?? report).slice(0, 2000)}`);
  process.exit(1);
}

// Each advisory once, from the packages it's found in. Packages that are only affected
// through another package name it as a string and add nothing new.
const serious = new Map();
for (const [name, entry] of Object.entries(report.vulnerabilities)) {
  for (const via of entry.via) {
    if (typeof via !== 'object' || !['high', 'critical'].includes(via.severity)) continue;
    const id = String(via.url ?? via.source).split('/').pop();
    serious.set(`${id} ${via.name}`, { id, name: via.name ?? name, range: via.range, severity: via.severity, title: via.title, url: via.url });
  }
}
const counted = (report.metadata.vulnerabilities.high ?? 0) + (report.metadata.vulnerabilities.critical ?? 0);
if (counted > 0 && serious.size === 0) {
  console.error(`npm audit counts ${counted} high or critical, but names no advisory to check.`);
  process.exit(1);
}

const excused = (advisory) =>
  exceptions.find((rule) => rule.advisory === advisory.id && rule.package === advisory.name && rule.range === advisory.range && today <= rule.until);

let failed = false;
for (const advisory of serious.values()) {
  const rule = excused(advisory);
  if (rule) {
    console.log(`Excepted until ${rule.until}: ${advisory.id} in ${advisory.name} ${advisory.range}. ${rule.reason}`);
  } else {
    failed = true;
    console.error(`${advisory.severity}: ${advisory.id} in ${advisory.name} ${advisory.range}: ${advisory.title} ${advisory.url ?? ''}`);
  }
}
for (const rule of exceptions) {
  if (![...serious.values()].some((advisory) => advisory.id === rule.advisory && advisory.name === rule.package)) {
    console.log(`No longer needed: the exception for ${rule.advisory} in ${rule.package}. Remove it.`);
  }
}
if (failed) process.exit(1);
console.log(serious.size ? 'npm audit: nothing high or critical beyond the reviewed exceptions.' : 'npm audit: nothing high or critical.');
