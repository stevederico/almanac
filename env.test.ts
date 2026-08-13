import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, symlinkSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { loadEnvFile, readConfig } from './env.ts';

describe('readConfig', () => {
  it('defaults port and name', () => {
    const cfg = readConfig({});
    assert.equal(cfg.port, 18788);
    assert.equal(cfg.host, '127.0.0.1');
    assert.equal(cfg.calName, 'My Calendar');
    assert.equal(cfg.tlsKey, '');
    assert.equal(cfg.tlsCert, '');
  });

  it('reads env values', () => {
    const cfg = readConfig({
      PORT: '9000',
      CAL_NAME: 'Agents',
      DB_PATH: '/tmp/c.db',
      FEED_TOKEN: 'feed',
      AGENT_KEY: 'key',
    });
    assert.equal(cfg.port, 9000);
    assert.equal(cfg.calName, 'Agents');
    assert.equal(cfg.dbPath, '/tmp/c.db');
    assert.equal(cfg.feedToken, 'feed');
    assert.equal(cfg.agentKey, 'key');
  });
});

describe('loadEnvFile', () => {
  it('loads unset keys only', () => {
    const dir = mkdtempSync(join(tmpdir(), 'mycal-env-'));
    const path = join(dir, '.env');
    writeFileSync(path, 'FEED_TOKEN=fromfile\nPORT=8123\n');
    delete process.env.FEED_TOKEN;
    process.env.PORT = '8000';
    loadEnvFile(path);
    assert.equal(process.env.FEED_TOKEN, 'fromfile');
    assert.equal(process.env.PORT, '8000');
    delete process.env.FEED_TOKEN;
    delete process.env.PORT;
  });

  it('refuses a symlink', () => {
    const dir = mkdtempSync(join(tmpdir(), 'mycal-env-'));
    const real = join(dir, 'real.env');
    const link = join(dir, '.env');
    writeFileSync(real, 'FEED_TOKEN=x\n');
    symlinkSync(real, link);
    assert.throws(() => loadEnvFile(link), /regular file/);
  });
});
