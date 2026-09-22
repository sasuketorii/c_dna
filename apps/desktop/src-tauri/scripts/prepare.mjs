import { mkdir, copyFile, cp, stat, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import { execFileSync } from 'node:child_process';
const root = fileURLToPath(new URL('../../../../', import.meta.url));
const crate = fileURLToPath(new URL('../', import.meta.url));
const target = execFileSync('rustc', ['-vV'], {encoding:'utf8'}).match(/^host: (.+)$/m)?.[1];
if (!target) throw new Error('rustc host missing');
const suffix = process.platform === 'win32' ? '.exe' : '';
const binary = resolve(root, `target/release/cdna${suffix}`);
await stat(binary);
await mkdir(resolve(crate,'binaries'),{recursive:true});
await copyFile(binary,resolve(crate,`binaries/cdna-${target}${suffix}`));
await rm(resolve(crate,'resources/ui'),{recursive:true,force:true});
await cp(resolve(crate,'../dist'),resolve(crate,'resources/ui'),{recursive:true,force:true});
console.log(`Prepared existing cdna backend sidecar for ${target} and UI assets`);

execFileSync('python3', [resolve(crate, 'scripts/bundle-learner.py')], {stdio: 'inherit'});
