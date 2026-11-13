// Made by WindowCraft Dev Team
import { execSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as path from 'node:path';
import * as os from 'node:os';

const workspaceRoot = process.cwd();
const cargoHome = process.env.CARGO_HOME || path.join(os.homedir(), '.cargo');
const registrySrcRoot = path.join(cargoHome, 'registry', 'src');

const targets = ['x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc'];
const workspaceCrates = new Set([
  'windowcraft-app',
  'windowcraft-cli',
  'windowcraft-config',
  'windowcraft-engine',
  'windowcraft-iso',
  'windowcraft-physics',
]);

let registryDirs = [];
if (fs.existsSync(registrySrcRoot)) {
  registryDirs = fs
    .readdirSync(registrySrcRoot, { withFileTypes: true })
    .filter((d) => d.isDirectory())
    .map((d) => path.join(registrySrcRoot, d.name));
}

const cratesMap = new Map();

for (const target of targets) {
  const cmd = `cargo tree --offline -p windowcraft-app -e normal --prefix none --format "{p}|{l}|{r}" --target ${target}`;
  const output = execSync(cmd, { cwd: workspaceRoot, encoding: 'utf8' });
  const lines = output.split(/\r?\n/);
  for (const line of lines) {
    if (!line.trim()) continue;
    const parts = line.split('|');
    if (parts.length < 2) continue;
    let pkg = parts[0].trim();
    const license = parts[1].trim() || 'Unspecified';

    pkg = pkg.replace(/\s+\(\*\)$/, '').trim();

    const tokens = pkg.split(/\s+/);
    const name = tokens[0];
    if (workspaceCrates.has(name) || pkg.includes('(')) {
      continue;
    }
    let version = tokens[1] || '';
    if (version.startsWith('v')) {
      version = version.slice(1);
    }
    const key = `${name}@${version}`;
    if (!cratesMap.has(key)) {
      cratesMap.set(key, { name, version, license });
    }
  }
}

const crates = Array.from(cratesMap.values()).sort((a, b) => {
  const c = a.name.localeCompare(b.name);
  return c !== 0 ? c : a.version.localeCompare(b.version);
});

const distinctLicenseTexts = new Map();
const noTextCrates = [];

for (const crate of crates) {
  const dirName = `${crate.name}-${crate.version}`;
  let foundDir = null;
  for (const regDir of registryDirs) {
    const candidate = path.join(regDir, dirName);
    if (fs.existsSync(candidate)) {
      foundDir = candidate;
      break;
    }
  }

  const licenseFiles = [];
  if (foundDir) {
    const files = fs.readdirSync(foundDir, { withFileTypes: true });
    for (const f of files) {
      if (f.isFile() && /^(LICENSE|COPYING|NOTICE)/i.test(f.name)) {
        licenseFiles.push(path.join(foundDir, f.name));
      }
    }
  }

  // mp3lame-sys builds the LAME C sources it bundles into the binary. Its
  // own LICENSE is the LGPL-3.0; LAME's COPYING (LGPL-2.0) sits one folder down.
  if (foundDir && crate.name === 'mp3lame-sys') {
    for (const f of fs.readdirSync(foundDir, { withFileTypes: true })) {
      const copying = path.join(foundDir, f.name, 'COPYING');
      if (f.isDirectory() && f.name.startsWith('lame') && fs.existsSync(copying)) {
        licenseFiles.push(copying);
      }
    }
  }

  // These crates publish no license file. iced keeps its text in the root
  // crate, and clipboard-win shares its author and license with error-code.
  // The zune crates offer Apache-2.0, which needs no copyright line.
  // symphonia 0.5 ships no MPL-2.0 text; symphonia 0.6.1 carries the same one.
  if (licenseFiles.length === 0) {
    const fallback = crate.name.startsWith('iced_')
      ? ['iced-0.14.0', 'LICENSE']
      : crate.name === 'clipboard-win'
        ? ['error-code-3.4.0', 'LICENSE']
        : crate.name.startsWith('zune-')
          ? ['cfg-if-1.0.5', 'LICENSE-APACHE']
          : crate.name.startsWith('symphonia')
            ? ['symphonia-0.6.1', 'LICENSE']
            : null;
    for (const regDir of registryDirs) {
      if (fallback && fs.existsSync(path.join(regDir, ...fallback))) {
        licenseFiles.push(path.join(regDir, ...fallback));
        break;
      }
    }
  }

  if (licenseFiles.length === 0) {
    noTextCrates.push(crate);
  } else {
    for (const lf of licenseFiles) {
      try {
        const content = fs.readFileSync(lf, 'utf8');
        const normalized = content.replace(/\r\n/g, '\n').trim();
        if (normalized.length === 0) continue;
        if (!distinctLicenseTexts.has(normalized)) {
          distinctLicenseTexts.set(normalized, {
            text: normalized,
            crates: [`${crate.name} ${crate.version}`],
            fileName: path.basename(lf),
          });
        } else {
          const entry = distinctLicenseTexts.get(normalized);
          const crateId = `${crate.name} ${crate.version}`;
          if (!entry.crates.includes(crateId)) {
            entry.crates.push(crateId);
          }
        }
      } catch (err) {
        // file unreadable
      }
    }
  }
}

// Build Markdown
const out = [];
out.push('<!-- Made by WindowCraft Dev Team -->');
out.push('# Third-Party Notices');
out.push('');
out.push('WindowCraft links the following third-party Rust crates.');
out.push('');
out.push('## Crates');
out.push('');
out.push('| Crate | Version | License |');
out.push('| :--- | :--- | :--- |');
for (const c of crates) {
  out.push(`| ${c.name} | ${c.version} | ${c.license} |`);
}
out.push('');

if (noTextCrates.length > 0) {
  out.push('## Crates with No Notice File in Package');
  out.push('');
  out.push('The following crates include no LICENSE, COPYING, or NOTICE file in their package archive:');
  out.push('');
  out.push('| Crate | Version | License |');
  out.push('| :--- | :--- | :--- |');
  for (const c of noTextCrates) {
    out.push(`| ${c.name} | ${c.version} | ${c.license} |`);
  }
  out.push('');
}

out.push('## License Texts');
out.push('');

let textIndex = 1;
for (const entry of distinctLicenseTexts.values()) {
  out.push(`### License ${textIndex}`);
  out.push('');
  out.push(`Used by: ${entry.crates.join(', ')}`);
  out.push('');
  out.push('```');
  out.push(entry.text);
  out.push('```');
  out.push('');
  textIndex++;
}

const outputPath = path.join(workspaceRoot, 'THIRD-PARTY-NOTICES.md');
fs.writeFileSync(outputPath, out.join('\n'), 'utf8');

console.log(`Processed ${crates.length} crates across both targets.`);
console.log(`Found ${distinctLicenseTexts.size} distinct license texts.`);
console.log(`Crates with no license text found: ${noTextCrates.length}`);
for (const c of noTextCrates) {
  console.log(`  - ${c.name} ${c.version} (${c.license})`);
}
