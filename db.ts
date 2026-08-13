import { DatabaseSync } from 'node:sqlite';
import { mkdirSync } from 'node:fs';
import { dirname } from 'node:path';
import { randomBytes, randomUUID } from 'node:crypto';
import { nextDate } from './ics.ts';

export type CalendarRow = {
  id: string;
  feedToken: string;
  agentKey: string;
  name: string;
  createdAt: string;
};

export type EventRow = {
  uid: string;
  summary: string;
  description: string;
  location: string;
  dtstart: string;
  dtend: string;
  allDay: boolean;
  transparent: boolean;
  sequence: number;
  createdAt: string;
  updatedAt: string;
};

export type EventInput = {
  uid?: string;
  summary: string;
  description?: string;
  location?: string;
  start: string;
  end?: string;
  allDay?: boolean;
  transparent?: boolean;
};

export type EventPatch = {
  summary?: string;
  description?: string;
  location?: string;
  start?: string;
  end?: string;
  allDay?: boolean;
  transparent?: boolean;
};

const UID_RE = /^[A-Za-z0-9._@-]{1,200}$/;
const YMD_RE = /^\d{4}-\d{2}-\d{2}$/;

function tableNames(db: DatabaseSync): string[] {
  const rows = db.prepare(`SELECT name FROM sqlite_master WHERE type = 'table'`).all();
  return rows.flatMap((r) => {
    if (!isRecord(r) || typeof r.name !== 'string') return [];
    return [r.name];
  });
}

function columnNames(db: DatabaseSync, table: string): string[] {
  const rows = db.prepare(`PRAGMA table_info(${table})`).all();
  return rows.flatMap((r) => {
    if (!isRecord(r) || typeof r.name !== 'string') return [];
    return [r.name];
  });
}

function migrateEvents(db: DatabaseSync): void {
  const tables = tableNames(db);
  if (!tables.includes('events')) {
    db.exec(`
      CREATE TABLE events (
        calendar_id TEXT NOT NULL,
        uid TEXT NOT NULL,
        summary TEXT NOT NULL,
        description TEXT NOT NULL DEFAULT '',
        location TEXT NOT NULL DEFAULT '',
        dtstart TEXT NOT NULL,
        dtend TEXT NOT NULL,
        all_day INTEGER NOT NULL DEFAULT 0,
        transparent INTEGER NOT NULL DEFAULT 1,
        sequence INTEGER NOT NULL DEFAULT 0,
        created_at TEXT NOT NULL,
        updated_at TEXT NOT NULL,
        PRIMARY KEY (calendar_id, uid)
      );
      CREATE INDEX IF NOT EXISTS idx_events_cal_start ON events(calendar_id, dtstart);
    `);
    return;
  }
  const cols = columnNames(db, 'events');
  if (cols.includes('calendar_id')) return;
  db.exec(`
    CREATE TABLE events_new (
      calendar_id TEXT NOT NULL,
      uid TEXT NOT NULL,
      summary TEXT NOT NULL,
      description TEXT NOT NULL DEFAULT '',
      location TEXT NOT NULL DEFAULT '',
      dtstart TEXT NOT NULL,
      dtend TEXT NOT NULL,
      all_day INTEGER NOT NULL DEFAULT 0,
      transparent INTEGER NOT NULL DEFAULT 1,
      sequence INTEGER NOT NULL DEFAULT 0,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL,
      PRIMARY KEY (calendar_id, uid)
    );
    INSERT INTO events_new (
      calendar_id, uid, summary, description, location, dtstart, dtend,
      all_day, transparent, sequence, created_at, updated_at
    )
    SELECT 'home', uid, summary, description, location, dtstart, dtend,
           all_day, transparent, sequence, created_at, updated_at
    FROM events;
    DROP TABLE events;
    ALTER TABLE events_new RENAME TO events;
    CREATE INDEX IF NOT EXISTS idx_events_cal_start ON events(calendar_id, dtstart);
  `);
}

export function openDb(dbPath: string): DatabaseSync {
  mkdirSync(dirname(dbPath), { recursive: true });
  const db = new DatabaseSync(dbPath);
  db.exec(`
    CREATE TABLE IF NOT EXISTS calendars (
      id TEXT PRIMARY KEY,
      feed_token TEXT NOT NULL UNIQUE,
      agent_key TEXT NOT NULL,
      name TEXT NOT NULL,
      created_at TEXT NOT NULL
    );
  `);
  migrateEvents(db);
  return db;
}

function secret(): string {
  return randomBytes(24).toString('hex');
}

function calendarFrom(raw: unknown): CalendarRow | null {
  if (!isRecord(raw)) return null;
  if (typeof raw.id !== 'string') return null;
  if (typeof raw.feed_token !== 'string') return null;
  if (typeof raw.agent_key !== 'string') return null;
  if (typeof raw.name !== 'string') return null;
  if (typeof raw.created_at !== 'string') return null;
  return {
    id: raw.id,
    feedToken: raw.feed_token,
    agentKey: raw.agent_key,
    name: raw.name,
    createdAt: raw.created_at,
  };
}

export function getCalendar(db: DatabaseSync, id: string): CalendarRow | null {
  const raw = db.prepare(`
    SELECT id, feed_token, agent_key, name, created_at
    FROM calendars WHERE id = ?
  `).get(id);
  return calendarFrom(raw);
}

export function getCalendarByFeedToken(db: DatabaseSync, token: string): CalendarRow | null {
  const raw = db.prepare(`
    SELECT id, feed_token, agent_key, name, created_at
    FROM calendars WHERE feed_token = ?
  `).get(token);
  return calendarFrom(raw);
}

export function createCalendar(
  db: DatabaseSync,
  input: { name?: string; id?: string; feedToken?: string; agentKey?: string } = {},
): CalendarRow | { error: string } {
  const name = (input.name ?? 'Almanac').trim() || 'Almanac';
  if (name.length > 80) return { error: 'name is too long' };
  const id = input.id ?? `cal_${randomBytes(8).toString('hex')}`;
  const now = new Date().toISOString();
  db.prepare(`
    INSERT INTO calendars (id, feed_token, agent_key, name, created_at)
    VALUES (?, ?, ?, ?, ?)
  `).run(id, input.feedToken ?? secret(), input.agentKey ?? secret(), name, now);
  const saved = getCalendar(db, id);
  if (!saved) return { error: 'failed to create calendar' };
  return saved;
}

/** Keep a pre-existing env feed working as calendar `home`. */
export function ensureHomeCalendar(
  db: DatabaseSync,
  home: { feedToken: string; agentKey: string; name: string },
): CalendarRow | { error: string } {
  const existing = getCalendar(db, 'home');
  if (existing) return existing;
  const byToken = getCalendarByFeedToken(db, home.feedToken);
  if (byToken) return byToken;
  return createCalendar(db, {
    id: 'home',
    name: home.name,
    feedToken: home.feedToken,
    agentKey: home.agentKey,
  });
}

export function jsonCalendar(
  row: CalendarRow,
  base: string,
): Record<string, unknown> {
  const origin = base.replace(/\/$/, '');
  return {
    id: row.id,
    name: row.name,
    subscribe: `${origin}/feed/${row.feedToken}.ics`,
    write: `${origin}/v1/c/${row.id}/events`,
    key: row.agentKey,
    createdAt: row.createdAt,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function asString(value: unknown): string | undefined {
  return typeof value === 'string' ? value : undefined;
}

function asBool(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined;
}

function parseInstant(value: string): string | null {
  const d = new Date(value);
  if (Number.isNaN(d.getTime())) return null;
  return d.toISOString();
}

function addHour(iso: string): string {
  return new Date(new Date(iso).getTime() + 60 * 60 * 1000).toISOString();
}

type Times = { dtstart: string; dtend: string; allDay: boolean };

function resolveTimes(start: string, end: string | undefined, allDay: boolean): Times | { error: string } {
  if (allDay || YMD_RE.test(start)) {
    if (!YMD_RE.test(start)) return { error: 'all-day start must be YYYY-MM-DD' };
    const endYmd = end ?? nextDate(start);
    if (!YMD_RE.test(endYmd)) return { error: 'all-day end must be YYYY-MM-DD' };
    if (endYmd <= start) return { error: 'end must be after start' };
    return { dtstart: start, dtend: endYmd, allDay: true };
  }
  const dtstart = parseInstant(start);
  if (!dtstart) return { error: 'start must be an ISO-8601 datetime' };
  const dtend = end ? parseInstant(end) : addHour(dtstart);
  if (!dtend) return { error: 'end must be an ISO-8601 datetime' };
  if (dtend <= dtstart) return { error: 'end must be after start' };
  return { dtstart, dtend, allDay: false };
}

function asInt(value: unknown): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'bigint') return Number(value);
  return null;
}

function rowFrom(raw: unknown): EventRow | null {
  if (!isRecord(raw)) return null;
  if (typeof raw.uid !== 'string') return null;
  if (typeof raw.summary !== 'string') return null;
  if (typeof raw.dtstart !== 'string') return null;
  if (typeof raw.dtend !== 'string') return null;
  if (typeof raw.created_at !== 'string') return null;
  if (typeof raw.updated_at !== 'string') return null;
  const sequence = asInt(raw.sequence);
  if (sequence === null) return null;
  return {
    uid: raw.uid,
    summary: raw.summary,
    description: typeof raw.description === 'string' ? raw.description : '',
    location: typeof raw.location === 'string' ? raw.location : '',
    dtstart: raw.dtstart,
    dtend: raw.dtend,
    allDay: asInt(raw.all_day) === 1,
    transparent: asInt(raw.transparent) === 1,
    sequence,
    createdAt: raw.created_at,
    updatedAt: raw.updated_at,
  };
}

/** Parse and validate a create/upsert body. */
export function parseEventInput(body: unknown): EventInput | { error: string } {
  if (!isRecord(body)) return { error: 'body must be an object' };
  const summary = asString(body.summary)?.trim();
  if (!summary) return { error: 'summary is required' };
  if (summary.length > 512) return { error: 'summary is too long' };
  const start = asString(body.start)?.trim();
  if (!start) return { error: 'start is required' };
  const uid = asString(body.uid)?.trim();
  if (uid !== undefined && !UID_RE.test(uid)) {
    return { error: 'uid must be 1-200 chars: letters, digits, . _ @ -' };
  }
  const description = asString(body.description);
  const location = asString(body.location);
  if (description !== undefined && description.length > 4000) {
    return { error: 'description is too long' };
  }
  if (location !== undefined && location.length > 512) {
    return { error: 'location is too long' };
  }
  const input: EventInput = { summary, start };
  if (uid !== undefined) input.uid = uid;
  if (description !== undefined) input.description = description;
  if (location !== undefined) input.location = location;
  if (asString(body.end) !== undefined) input.end = asString(body.end);
  if (asBool(body.allDay) !== undefined) input.allDay = asBool(body.allDay);
  if (asBool(body.transparent) !== undefined) input.transparent = asBool(body.transparent);
  return input;
}

/** Parse a PATCH body. At least one field required. */
export function parseEventPatch(body: unknown): EventPatch | { error: string } {
  if (!isRecord(body)) return { error: 'body must be an object' };
  const patch: EventPatch = {};
  const summary = asString(body.summary);
  if (summary !== undefined) {
    const trimmed = summary.trim();
    if (!trimmed) return { error: 'summary cannot be empty' };
    if (trimmed.length > 512) return { error: 'summary is too long' };
    patch.summary = trimmed;
  }
  const start = asString(body.start);
  if (start !== undefined) patch.start = start.trim();
  const end = asString(body.end);
  if (end !== undefined) patch.end = end.trim();
  const description = asString(body.description);
  if (description !== undefined) {
    if (description.length > 4000) return { error: 'description is too long' };
    patch.description = description;
  }
  const location = asString(body.location);
  if (location !== undefined) {
    if (location.length > 512) return { error: 'location is too long' };
    patch.location = location;
  }
  const allDay = asBool(body.allDay);
  if (allDay !== undefined) patch.allDay = allDay;
  const transparent = asBool(body.transparent);
  if (transparent !== undefined) patch.transparent = transparent;
  if (Object.keys(patch).length === 0) return { error: 'no fields to update' };
  return patch;
}

export function listEvents(db: DatabaseSync, calendarId: string): EventRow[] {
  const rows = db.prepare(`
    SELECT uid, summary, description, location, dtstart, dtend,
           all_day, transparent, sequence, created_at, updated_at
    FROM events
    WHERE calendar_id = ?
    ORDER BY dtstart ASC
  `).all(calendarId);
  return rows.flatMap((r) => {
    const row = rowFrom(r);
    return row ? [row] : [];
  });
}

export function getEvent(db: DatabaseSync, calendarId: string, uid: string): EventRow | null {
  const raw = db.prepare(`
    SELECT uid, summary, description, location, dtstart, dtend,
           all_day, transparent, sequence, created_at, updated_at
    FROM events
    WHERE calendar_id = ? AND uid = ?
  `).get(calendarId, uid);
  return rowFrom(raw);
}

export function upsertEvent(
  db: DatabaseSync,
  calendarId: string,
  input: EventInput,
): EventRow | { error: string } {
  const allDay = input.allDay === true || YMD_RE.test(input.start);
  const times = resolveTimes(input.start, input.end, allDay);
  if ('error' in times) return times;
  const now = new Date().toISOString();
  const uid = input.uid ?? `evt-${randomUUID()}`;
  if (!UID_RE.test(uid)) return { error: 'uid must be 1-200 chars: letters, digits, . _ @ -' };
  const existing = getEvent(db, calendarId, uid);
  const summary = input.summary;
  const description = input.description ?? existing?.description ?? '';
  const location = input.location ?? existing?.location ?? '';
  const transparent = input.transparent ?? existing?.transparent ?? true;
  if (existing) {
    db.prepare(`
      UPDATE events
      SET summary = ?, description = ?, location = ?, dtstart = ?, dtend = ?,
          all_day = ?, transparent = ?, sequence = sequence + 1, updated_at = ?
      WHERE calendar_id = ? AND uid = ?
    `).run(
      summary,
      description,
      location,
      times.dtstart,
      times.dtend,
      times.allDay ? 1 : 0,
      transparent ? 1 : 0,
      now,
      calendarId,
      uid,
    );
  } else {
    db.prepare(`
      INSERT INTO events (
        calendar_id, uid, summary, description, location, dtstart, dtend,
        all_day, transparent, sequence, created_at, updated_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)
    `).run(
      calendarId,
      uid,
      summary,
      description,
      location,
      times.dtstart,
      times.dtend,
      times.allDay ? 1 : 0,
      transparent ? 1 : 0,
      now,
      now,
    );
  }
  const saved = getEvent(db, calendarId, uid);
  if (!saved) return { error: 'failed to save event' };
  return saved;
}

export function patchEvent(
  db: DatabaseSync,
  calendarId: string,
  uid: string,
  patch: EventPatch,
): EventRow | { error: string } {
  const existing = getEvent(db, calendarId, uid);
  if (!existing) return { error: 'not found' };
  const start = patch.start ?? existing.dtstart;
  const allDay = patch.allDay ?? existing.allDay;
  const end = patch.end ?? existing.dtend;
  const times = resolveTimes(start, end, allDay);
  if ('error' in times) return times;
  const now = new Date().toISOString();
  db.prepare(`
    UPDATE events
    SET summary = ?, description = ?, location = ?, dtstart = ?, dtend = ?,
        all_day = ?, transparent = ?, sequence = sequence + 1, updated_at = ?
    WHERE calendar_id = ? AND uid = ?
  `).run(
    patch.summary ?? existing.summary,
    patch.description ?? existing.description,
    patch.location ?? existing.location,
    times.dtstart,
    times.dtend,
    times.allDay ? 1 : 0,
    (patch.transparent ?? existing.transparent) ? 1 : 0,
    now,
    calendarId,
    uid,
  );
  const saved = getEvent(db, calendarId, uid);
  if (!saved) return { error: 'failed to save event' };
  return saved;
}

export function deleteEvent(db: DatabaseSync, calendarId: string, uid: string): boolean {
  const result = db.prepare('DELETE FROM events WHERE calendar_id = ? AND uid = ?').run(
    calendarId,
    uid,
  );
  return Number(result.changes) > 0;
}

export function jsonEvent(row: EventRow): Record<string, unknown> {
  return {
    uid: row.uid,
    summary: row.summary,
    description: row.description,
    location: row.location,
    start: row.dtstart,
    end: row.dtend,
    allDay: row.allDay,
    transparent: row.transparent,
    sequence: row.sequence,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
  };
}
