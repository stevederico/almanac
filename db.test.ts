import { describe, it, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import {
  deleteEvent,
  getEvent,
  listEvents,
  openDb,
  parseEventInput,
  patchEvent,
  upsertEvent,
} from './db.ts';
import type { DatabaseSync } from 'node:sqlite';

let db: DatabaseSync;

beforeEach(() => {
  const dir = mkdtempSync(join(tmpdir(), 'mycal-db-'));
  db = openDb(join(dir, 'calendar.db'));
});

describe('parseEventInput', () => {
  it('requires summary and start', () => {
    const miss = parseEventInput({ summary: 'x' });
    assert.ok('error' in miss);
    const ok = parseEventInput({ summary: 'Dinner', start: '2026-08-16T19:00:00-07:00' });
    assert.ok(!('error' in ok));
    assert.equal(ok.summary, 'Dinner');
  });

  it('rejects a bad uid', () => {
    const bad = parseEventInput({
      uid: 'has spaces',
      summary: 'x',
      start: '2026-08-16T19:00:00Z',
    });
    assert.ok('error' in bad);
  });
});

describe('upsertEvent', () => {
  it('creates then bumps sequence on the same uid', () => {
    const a = upsertEvent(db, {
      uid: 'dinner-1',
      summary: 'Dinner',
      start: '2026-08-16T19:00:00.000Z',
      end: '2026-08-16T21:00:00.000Z',
    });
    assert.ok(!('error' in a));
    assert.equal(a.sequence, 0);
    const b = upsertEvent(db, {
      uid: 'dinner-1',
      summary: 'Dinner moved',
      start: '2026-08-16T20:00:00.000Z',
    });
    assert.ok(!('error' in b));
    assert.equal(b.summary, 'Dinner moved');
    assert.equal(b.sequence, 1);
    assert.equal(listEvents(db).length, 1);
  });

  it('treats YYYY-MM-DD as all-day', () => {
    const ev = upsertEvent(db, { summary: 'Off', start: '2026-08-20' });
    assert.ok(!('error' in ev));
    assert.equal(ev.allDay, true);
    assert.equal(ev.dtend, '2026-08-21');
  });

  it('rejects end before start', () => {
    const ev = upsertEvent(db, {
      summary: 'Bad',
      start: '2026-08-16T19:00:00Z',
      end: '2026-08-16T18:00:00Z',
    });
    assert.ok('error' in ev);
  });
});

describe('patch and delete', () => {
  it('patches one field and deletes', () => {
    const created = upsertEvent(db, {
      uid: 'n1',
      summary: 'Note',
      start: '2026-08-16T12:00:00Z',
    });
    assert.ok(!('error' in created));
    const patched = patchEvent(db, 'n1', { location: 'Oracle Park' });
    assert.ok(!('error' in patched));
    assert.equal(patched.location, 'Oracle Park');
    assert.equal(patched.sequence, 1);
    assert.equal(deleteEvent(db, 'n1'), true);
    assert.equal(getEvent(db, 'n1'), null);
  });
});
