import { serve } from '@hono/node-server';
import { Hono } from 'hono';
import { timingSafeEqual } from 'node:crypto';
import { createServer } from 'node:https';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import type { DatabaseSync } from 'node:sqlite';
import {
  deleteEvent,
  getEvent,
  jsonEvent,
  listEvents,
  openDb,
  parseEventInput,
  parseEventPatch,
  patchEvent,
  upsertEvent,
  type EventRow,
} from './db.ts';
import { renderCalendar } from './ics.ts';
import { loadEnvFile, readConfig } from './env.ts';

export type AppOptions = {
  db: DatabaseSync;
  feedToken: string;
  agentKey: string;
  calName: string;
};

function safeEqual(given: string, expected: string): boolean {
  if (expected.length === 0) return false;
  const a = Buffer.from(given);
  const b = Buffer.from(expected);
  if (a.length !== b.length) return false;
  return timingSafeEqual(a, b);
}

function bearer(header: string | undefined): string {
  if (!header) return '';
  const prefix = 'Bearer ';
  if (!header.startsWith(prefix)) return '';
  return header.slice(prefix.length);
}

function feedTokenOf(raw: string): string {
  return raw.replace(/\.ics$/i, '');
}

function requireAgent(
  c: { req: { header: (name: string) => string | undefined } },
  agentKey: string,
): boolean {
  return safeEqual(bearer(c.req.header('authorization')), agentKey);
}

function icsOf(calName: string, events: EventRow[]): string {
  return renderCalendar(
    calName,
    events.map((ev) => ({
      uid: ev.uid,
      summary: ev.summary,
      description: ev.description,
      location: ev.location,
      dtstart: ev.dtstart,
      dtend: ev.dtend,
      allDay: ev.allDay,
      transparent: ev.transparent,
      sequence: ev.sequence,
      updatedAt: ev.updatedAt,
    })),
  );
}

/** Hono app: public ICS feed + bearer agent API. */
export function createApp(opts: AppOptions): Hono {
  const app = new Hono();

  app.use(async (c, next) => {
    const t0 = Date.now();
    await next();
    const path = c.req.path.replace(opts.feedToken, '<token>');
    console.log(JSON.stringify({
      t: new Date().toISOString(),
      msg: 'req',
      method: c.req.method,
      path,
      status: c.res.status,
      ms: Date.now() - t0,
      ua: c.req.header('user-agent') ?? '',
    }));
  });

  app.get('/health', (c) => c.json({ ok: true }));

  app.get('/feed/:token', (c) => {
    const token = feedTokenOf(c.req.param('token'));
    if (!safeEqual(token, opts.feedToken)) {
      return c.json({ error: 'not found' }, 404);
    }
    const body = icsOf(opts.calName, listEvents(opts.db));
    return c.newResponse(body, 200, {
      'content-type': 'text/calendar; charset=utf-8',
      'cache-control': 'no-cache',
      'content-disposition': 'inline; filename="calendar.ics"',
    });
  });

  app.use('/v1/*', async (c, next) => {
    if (!requireAgent(c, opts.agentKey)) {
      return c.json({ error: 'unauthorized' }, 401);
    }
    await next();
  });

  app.get('/v1/events', (c) => {
    return c.json({ events: listEvents(opts.db).map(jsonEvent) });
  });

  app.get('/v1/events/:uid', (c) => {
    const row = getEvent(opts.db, c.req.param('uid'));
    if (!row) return c.json({ error: 'not found' }, 404);
    return c.json(jsonEvent(row));
  });

  app.post('/v1/events', async (c) => {
    const body: unknown = await c.req.json().catch(() => null);
    const input = parseEventInput(body);
    if ('error' in input) return c.json({ error: input.error }, 400);
    const saved = upsertEvent(opts.db, input);
    if ('error' in saved) return c.json({ error: saved.error }, 400);
    return c.json(jsonEvent(saved), saved.sequence === 0 ? 201 : 200);
  });

  app.put('/v1/events/:uid', async (c) => {
    const body: unknown = await c.req.json().catch(() => null);
    const input = parseEventInput(body);
    if ('error' in input) return c.json({ error: input.error }, 400);
    input.uid = c.req.param('uid');
    const saved = upsertEvent(opts.db, input);
    if ('error' in saved) return c.json({ error: saved.error }, 400);
    const status = saved.sequence === 0 ? 201 : 200;
    return c.json(jsonEvent(saved), status);
  });

  app.patch('/v1/events/:uid', async (c) => {
    const body: unknown = await c.req.json().catch(() => null);
    const patch = parseEventPatch(body);
    if ('error' in patch) return c.json({ error: patch.error }, 400);
    const saved = patchEvent(opts.db, c.req.param('uid'), patch);
    if ('error' in saved) {
      const status = saved.error === 'not found' ? 404 : 400;
      return c.json({ error: saved.error }, status);
    }
    return c.json(jsonEvent(saved));
  });

  app.delete('/v1/events/:uid', (c) => {
    if (!deleteEvent(opts.db, c.req.param('uid'))) {
      return c.json({ error: 'not found' }, 404);
    }
    return c.body(null, 204);
  });

  return app;
}

function isMain(): boolean {
  const entry = process.argv[1];
  if (!entry) return false;
  return fileURLToPath(import.meta.url) === resolve(entry);
}

if (isMain()) {
  loadEnvFile(new URL('./.env', import.meta.url).pathname);
  const cfg = readConfig();
  if (!cfg.feedToken || !cfg.agentKey) {
    console.error('FEED_TOKEN and AGENT_KEY are required');
    process.exit(1);
  }
  const app = createApp({
    db: openDb(cfg.dbPath),
    feedToken: cfg.feedToken,
    agentKey: cfg.agentKey,
    calName: cfg.calName,
  });
  const scheme = cfg.tlsKey && cfg.tlsCert ? 'https' : 'http';
  if (scheme === 'https') {
    serve({
      fetch: app.fetch,
      port: cfg.port,
      hostname: cfg.host,
      createServer,
      serverOptions: {
        key: readFileSync(cfg.tlsKey),
        cert: readFileSync(cfg.tlsCert),
      },
    });
  } else {
    serve({ fetch: app.fetch, port: cfg.port, hostname: cfg.host });
  }
  console.log(JSON.stringify({
    t: new Date().toISOString(),
    msg: 'listen',
    host: cfg.host,
    port: cfg.port,
    scheme,
    feed: `${scheme}://${cfg.host}:${cfg.port}/feed/<token>.ics`,
  }));
}
