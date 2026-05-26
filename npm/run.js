#!/usr/bin/env node
'use strict';

const path = require('path');
const fs = require('fs');
const { spawnSync } = require('child_process');

const binPath = path.join(__dirname, 'bin', 'bo');

if (!fs.existsSync(binPath)) {
  console.error(
    `bo binary not found at ${binPath}. ` +
    `Try reinstalling: npm install -g @skillicinski/bo`,
  );
  process.exit(1);
}

const result = spawnSync(binPath, process.argv.slice(2), { stdio: 'inherit' });

if (result.error) {
  throw result.error;
}

process.exit(result.status != null ? result.status : 1);
