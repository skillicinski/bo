'use strict';

const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

const pkg = require('./package.json');
const version = pkg.version;

const PLATFORM_MAP = {
  'darwin-x64': 'x86_64-apple-darwin',
  'darwin-arm64': 'aarch64-apple-darwin',
  'linux-x64': 'x86_64-unknown-linux-gnu',
};

const key = `${process.platform}-${process.arch}`;
const target = PLATFORM_MAP[key];

if (!target) {
  console.error(
    `bo doesn't provide a pre-built binary for ${process.platform}-${process.arch}. ` +
    `Build from source: cargo install bo`,
  );
  process.exit(0);
}

const binDir = path.join(__dirname, 'bin');
const binPath = path.join(binDir, 'bo');
const versionFile = path.join(binDir, '.bo-version');

// Re-download guard: skip if correct version is already cached
if (fs.existsSync(versionFile) && fs.existsSync(binPath)) {
  const cachedVersion = fs.readFileSync(versionFile, 'utf8').trim();
  if (cachedVersion === version) {
    return;
  }
}

fs.mkdirSync(binDir, { recursive: true });

const url = `https://github.com/skillicinski/bo/releases/download/v${version}/bo-${target}.tar.gz`;
const tarball = path.join(binDir, '.bo-tmp.tar.gz');

try {
  execSync(`curl -fsSL --retry 2 -o "${tarball}" "${url}"`, { stdio: 'inherit' });
} catch (err) {
  try { fs.unlinkSync(tarball); } catch (_) {}
  console.error(`failed to download bo binary from ${url}: curl exit ${err.status}`);
  process.exit(1);
}

try {
  execSync(`tar xzf "${tarball}" -C "${binDir}"`, { stdio: 'inherit' });
  fs.unlinkSync(tarball);
} catch (err) {
  try { fs.unlinkSync(tarball); } catch (_) {}
  console.error('failed to extract bo binary');
  process.exit(1);
}

try {
  fs.chmodSync(binPath, 0o755);
  fs.writeFileSync(versionFile, version);
  console.error(`bo v${version} installed (${target})`);
} catch (err) {
  console.error(`failed to install bo: ${err.message}`);
  process.exit(1);
}
