import { existsSync, lstatSync, readFileSync } from 'node:fs';

/**
 * Load KEY=value pairs from a regular file into process.env.
 * Existing env wins. Symlinks are refused.
 */
export function loadEnvFile(path: string): void {
  if (!existsSync(path)) return;
  if (lstatSync(path).isSymbolicLink()) {
    throw new Error(`${path} must be a regular file, not a symlink`);
  }
  for (const raw of readFileSync(path, 'utf8').split('\n')) {
    const line = raw.trim();
    if (line === '' || line.startsWith('#')) continue;
    const eq = line.indexOf('=');
    if (eq <= 0) continue;
    const key = line.slice(0, eq).trim();
    if (!/^[A-Z_][A-Z0-9_]*$/.test(key)) continue;
    if (process.env[key] !== undefined) continue;
    let value = line.slice(eq + 1).trim();
    if (
      (value.startsWith('"') && value.endsWith('"'))
      || (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }
    process.env[key] = value;
  }
}

export type Config = {
  port: number;
  host: string;
  calName: string;
  dbPath: string;
  feedToken: string;
  agentKey: string;
  tlsKey: string;
  tlsCert: string;
};

const DEFAULT_PORT = 18788;

/** Read runtime config. Empty secrets are allowed here; server refuses to listen. */
export function readConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const portRaw = env.PORT ?? String(DEFAULT_PORT);
  const port = Number.parseInt(portRaw, 10);
  const host = (env.HOST ?? 'localhost').trim() || 'localhost';
  return {
    port: Number.isFinite(port) && port > 0 ? port : DEFAULT_PORT,
    host,
    calName: (env.CAL_NAME ?? 'Almanac').trim() || 'Almanac',
    dbPath: env.DB_PATH ?? './data/calendar.db',
    feedToken: env.FEED_TOKEN ?? '',
    agentKey: env.AGENT_KEY ?? '',
    tlsKey: env.TLS_KEY ?? '',
    tlsCert: env.TLS_CERT ?? '',
  };
}
