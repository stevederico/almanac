import { serve } from '@hono/node-server';
import { serveStatic } from '@hono/node-server/serve-static';
import { Hono } from 'hono';
import { timingSafeEqual } from 'node:crypto';
import { createServer } from 'node:https';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { resolve } from 'node:path';
import type { DatabaseSync } from 'node:sqlite';
import {
  createCalendar,
  deleteEvent,
  ensureHomeCalendar,
  getCalendar,
  getCalendarByFeedToken,
  getEvent,
  jsonCalendar,
  jsonEvent,
  listEvents,
  openDb,
  parseEventInput,
  parseEventPatch,
  patchEvent,
  upsertEvent,
  type CalendarRow,
  type EventRow,
} from './db.ts';
import { renderCalendar } from './ics.ts';
import { loadEnvFile, readConfig } from './env.ts';
import { htmlCreated, htmlHome, jsonIndex, llmsTxt } from './landing.ts';

export type AppOptions = {
  db: DatabaseSync;
  publicBase?: string;
  home?: { feedToken: string; agentKey: string; name: string };
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

function requestBase(c: { req: { header: (name: string) => string | undefined } }, fallback: string): string {
  if (fallback) return fallback.replace(/\/$/, '');
  const proto = c.req.header('x-forwarded-proto') || 'http';
  const host = c.req.header('host') || 'localhost';
  return `${proto}://${host}`;
}

function wantsHtml(c: { req: { header: (name: string) => string | undefined } }): boolean {
  const accept = c.req.header('accept') ?? '';
  return accept.includes('text/html') && !accept.includes('application/json');
}

function calendarAuth(db: DatabaseSync, id: string, header: string | undefined): CalendarRow | null {
  const cal = getCalendar(db, id);
  if (!cal) return null;
  if (!safeEqual(bearer(header), cal.agentKey)) return null;
  return cal;
}

/** Hono app: provision calendars, public ICS feeds, bearer writes. */
export function createApp(opts: AppOptions): Hono {
  if (opts.home) {
    const home = ensureHomeCalendar(opts.db, opts.home);
    if ('error' in home) throw new Error(home.error);
  }

  const app = new Hono();

  app.use(async (c, next) => {
    const t0 = Date.now();
    await next();
    const path = c.req.path.replace(/\/feed\/[^/]+/i, '/feed/<token>');
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

  app.use('/mascot.webp', serveStatic({ root: './public' }));
  app.use('/og.png', serveStatic({ root: './public' }));

  app.get('/health', (c) => c.json({ ok: true }));

  app.get('/robots.txt', (c) => {
    return c.text('User-agent: *\nDisallow: /\n', 200, {
      'content-type': 'text/plain; charset=utf-8',
    });
  });

  app.get('/llms.txt', (c) => {
    return c.text(llmsTxt(requestBase(c, opts.publicBase ?? '')), 200, {
      'content-type': 'text/plain; charset=utf-8',
    });
  });

  app.get('/', (c) => {
    const base = requestBase(c, opts.publicBase ?? '');
    if (wantsHtml(c)) {
      return c.html(htmlHome(base));
    }
    return c.json(jsonIndex(base));
  });

  app.post('/calendars', async (c) => {
    const type = c.req.header('content-type') ?? '';
    let name: string | undefined;
    if (type.includes('application/json')) {
      const body: unknown = await c.req.json().catch(() => null);
      if (body && typeof body === 'object' && !Array.isArray(body) && 'name' in body) {
        const raw = body.name;
        if (typeof raw === 'string') name = raw;
      }
    } else if (type.includes('application/x-www-form-urlencoded') || type.includes('multipart/form-data')) {
      const form = await c.req.parseBody();
      const raw = form.name;
      if (typeof raw === 'string') name = raw;
    }
    const saved = createCalendar(opts.db, { name });
    if ('error' in saved) return c.json({ error: saved.error }, 400);
    const base = requestBase(c, opts.publicBase ?? '');
    if (wantsHtml(c) || type.includes('application/x-www-form-urlencoded')) {
      return c.html(htmlCreated(saved, base), 201);
    }
    return c.json(jsonCalendar(saved, base), 201);
  });

  app.get('/feed/:token', (c) => {
    const token = feedTokenOf(c.req.param('token'));
    const cal = getCalendarByFeedToken(opts.db, token);
    if (!cal || !safeEqual(token, cal.feedToken)) {
      return c.json({ error: 'not found' }, 404);
    }
    const body = icsOf(cal.name, listEvents(opts.db, cal.id));
    return c.newResponse(body, 200, {
      'content-type': 'text/calendar; charset=utf-8',
      'cache-control': 'no-cache',
      'content-disposition': 'inline; filename="calendar.ics"',
    });
  });

  app.get('/v1/c/:id', (c) => {
    const cal = calendarAuth(opts.db, c.req.param('id'), c.req.header('authorization'));
    if (!cal) return c.json({ error: 'unauthorized' }, 401);
    return c.json(jsonCalendar(cal, requestBase(c, opts.publicBase ?? '')));
  });

  app.get('/v1/c/:id/events', (c) => {
    const cal = calendarAuth(opts.db, c.req.param('id'), c.req.header('authorization'));
    if (!cal) return c.json({ error: 'unauthorized' }, 401);
    return c.json({ events: listEvents(opts.db, cal.id).map(jsonEvent) });
  });

  app.get('/v1/c/:id/events/:uid', (c) => {
    const cal = calendarAuth(opts.db, c.req.param('id'), c.req.header('authorization'));
    if (!cal) return c.json({ error: 'unauthorized' }, 401);
    const row = getEvent(opts.db, cal.id, c.req.param('uid'));
    if (!row) return c.json({ error: 'not found' }, 404);
    return c.json(jsonEvent(row));
  });

  app.post('/v1/c/:id/events', async (c) => {
    const cal = calendarAuth(opts.db, c.req.param('id'), c.req.header('authorization'));
    if (!cal) return c.json({ error: 'unauthorized' }, 401);
    const body: unknown = await c.req.json().catch(() => null);
    const input = parseEventInput(body);
    if ('error' in input) return c.json({ error: input.error }, 400);
    const saved = upsertEvent(opts.db, cal.id, input);
    if ('error' in saved) return c.json({ error: saved.error }, 400);
    return c.json(jsonEvent(saved), saved.sequence === 0 ? 201 : 200);
  });

  app.put('/v1/c/:id/events/:uid', async (c) => {
    const cal = calendarAuth(opts.db, c.req.param('id'), c.req.header('authorization'));
    if (!cal) return c.json({ error: 'unauthorized' }, 401);
    const body: unknown = await c.req.json().catch(() => null);
    const input = parseEventInput(body);
    if ('error' in input) return c.json({ error: input.error }, 400);
    input.uid = c.req.param('uid');
    const saved = upsertEvent(opts.db, cal.id, input);
    if ('error' in saved) return c.json({ error: saved.error }, 400);
    return c.json(jsonEvent(saved), saved.sequence === 0 ? 201 : 200);
  });

  app.patch('/v1/c/:id/events/:uid', async (c) => {
    const cal = calendarAuth(opts.db, c.req.param('id'), c.req.header('authorization'));
    if (!cal) return c.json({ error: 'unauthorized' }, 401);
    const body: unknown = await c.req.json().catch(() => null);
    const patch = parseEventPatch(body);
    if ('error' in patch) return c.json({ error: patch.error }, 400);
    const saved = patchEvent(opts.db, cal.id, c.req.param('uid'), patch);
    if ('error' in saved) {
      const status = saved.error === 'not found' ? 404 : 400;
      return c.json({ error: saved.error }, status);
    }
    return c.json(jsonEvent(saved));
  });

  app.delete('/v1/c/:id/events/:uid', (c) => {
    const cal = calendarAuth(opts.db, c.req.param('id'), c.req.header('authorization'));
    if (!cal) return c.json({ error: 'unauthorized' }, 401);
    if (!deleteEvent(opts.db, cal.id, c.req.param('uid'))) {
      return c.json({ error: 'not found' }, 404);
    }
    return c.body(null, 204);
  });

  // yagni: /v1/events aliases home calendar so existing clients keep working
  const homeRoutes = new Hono();
  homeRoutes.use(async (c, next) => {
    const home = getCalendar(opts.db, 'home');
    if (!home || !safeEqual(bearer(c.req.header('authorization')), home.agentKey)) {
      return c.json({ error: 'unauthorized' }, 401);
    }
    await next();
  });
  homeRoutes.get('/events', (c) => {
    return c.json({ events: listEvents(opts.db, 'home').map(jsonEvent) });
  });
  homeRoutes.get('/events/:uid', (c) => {
    const row = getEvent(opts.db, 'home', c.req.param('uid'));
    if (!row) return c.json({ error: 'not found' }, 404);
    return c.json(jsonEvent(row));
  });
  homeRoutes.post('/events', async (c) => {
    const body: unknown = await c.req.json().catch(() => null);
    const input = parseEventInput(body);
    if ('error' in input) return c.json({ error: input.error }, 400);
    const saved = upsertEvent(opts.db, 'home', input);
    if ('error' in saved) return c.json({ error: saved.error }, 400);
    return c.json(jsonEvent(saved), saved.sequence === 0 ? 201 : 200);
  });
  homeRoutes.put('/events/:uid', async (c) => {
    const body: unknown = await c.req.json().catch(() => null);
    const input = parseEventInput(body);
    if ('error' in input) return c.json({ error: input.error }, 400);
    input.uid = c.req.param('uid');
    const saved = upsertEvent(opts.db, 'home', input);
    if ('error' in saved) return c.json({ error: saved.error }, 400);
    return c.json(jsonEvent(saved), saved.sequence === 0 ? 201 : 200);
  });
  homeRoutes.patch('/events/:uid', async (c) => {
    const body: unknown = await c.req.json().catch(() => null);
    const patch = parseEventPatch(body);
    if ('error' in patch) return c.json({ error: patch.error }, 400);
    const saved = patchEvent(opts.db, 'home', c.req.param('uid'), patch);
    if ('error' in saved) {
      const status = saved.error === 'not found' ? 404 : 400;
      return c.json({ error: saved.error }, status);
    }
    return c.json(jsonEvent(saved));
  });
  homeRoutes.delete('/events/:uid', (c) => {
    if (!deleteEvent(opts.db, 'home', c.req.param('uid'))) {
      return c.json({ error: 'not found' }, 404);
    }
    return c.body(null, 204);
  });
  app.route('/v1', homeRoutes);

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
  const home = cfg.feedToken && cfg.agentKey
    ? { feedToken: cfg.feedToken, agentKey: cfg.agentKey, name: cfg.calName }
    : undefined;
  const app = createApp({
    db: openDb(cfg.dbPath),
    publicBase: process.env.PUBLIC_BASE,
    home,
  });
  const scheme = cfg.tlsKey && cfg.tlsCert ? 'https' : 'http';
  const binds = listenHosts(cfg.host);
  for (const hostname of binds) {
    if (scheme === 'https') {
      serve({
        fetch: app.fetch,
        port: cfg.port,
        hostname,
        createServer,
        serverOptions: {
          key: readFileSync(cfg.tlsKey),
          cert: readFileSync(cfg.tlsCert),
        },
      });
    } else {
      serve({ fetch: app.fetch, port: cfg.port, hostname });
    }
  }
  console.log(JSON.stringify({
    t: new Date().toISOString(),
    msg: 'listen',
    host: cfg.host,
    binds,
    port: cfg.port,
    scheme,
  }));
}

/** Loopback hostnames must listen on v4 and v6. `localhost` prefers ::1. */
function listenHosts(host: string): string[] {
  if (host === 'localhost' || host === '127.0.0.1' || host === '::1') {
    return ['127.0.0.1', '::1'];
  }
  return [host];
}
