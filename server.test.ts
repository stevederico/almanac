import { describe, it, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openDb } from './db.ts';
import { createApp } from './server.ts';

const FEED = 'feed-secret';
const KEY = 'agent-secret';

let app: ReturnType<typeof createApp>;

beforeEach(() => {
  const dir = mkdtempSync(join(tmpdir(), 'mycal-srv-'));
  app = createApp({
    db: openDb(join(dir, 'calendar.db')),
    feedToken: FEED,
    agentKey: KEY,
    calName: 'My Calendar',
  });
});

function auth(extra?: Record<string, string>): Record<string, string> {
  return { authorization: `Bearer ${KEY}`, ...extra };
}

describe('feed', () => {
  it('404s a wrong token', async () => {
    const res = await app.request('/feed/nope.ics');
    assert.equal(res.status, 404);
  });

  it('returns text/calendar for the real token', async () => {
    await app.request('/v1/events', {
      method: 'POST',
      headers: auth({ 'content-type': 'application/json' }),
      body: JSON.stringify({
        uid: 'giants-20260816',
        summary: 'Rockies @ Giants',
        start: '2026-08-16T13:05:00-07:00',
        location: 'Oracle Park',
      }),
    });
    const res = await app.request(`/feed/${FEED}.ics`);
    assert.equal(res.status, 200);
    assert.match(res.headers.get('content-type') ?? '', /text\/calendar/);
    const body = await res.text();
    assert.match(body, /BEGIN:VCALENDAR/);
    assert.match(body, /UID:giants-20260816/);
    assert.match(body, /SUMMARY:Rockies @ Giants/);
  });
});

describe('agent api', () => {
  it('rejects missing bearer', async () => {
    const res = await app.request('/v1/events');
    assert.equal(res.status, 401);
  });

  it('creates, lists, patches, deletes', async () => {
    const created = await app.request('/v1/events', {
      method: 'POST',
      headers: auth({ 'content-type': 'application/json' }),
      body: JSON.stringify({
        summary: 'Dentist',
        start: '2026-08-18T09:00:00-07:00',
        end: '2026-08-18T09:45:00-07:00',
      }),
    });
    assert.equal(created.status, 201);
    const createdBody: unknown = await created.json();
    assert.ok(createdBody && typeof createdBody === 'object' && 'uid' in createdBody);
    const uid = createdBody.uid;
    assert.equal(typeof uid, 'string');

    const listed = await app.request('/v1/events', { headers: auth() });
    assert.equal(listed.status, 200);
    const listBody: unknown = await listed.json();
    assert.ok(listBody && typeof listBody === 'object' && 'events' in listBody);
    const events = listBody.events;
    assert.ok(Array.isArray(events));
    assert.equal(events.length, 1);

    const patched = await app.request(`/v1/events/${uid}`, {
      method: 'PATCH',
      headers: auth({ 'content-type': 'application/json' }),
      body: JSON.stringify({ location: 'Fillmore' }),
    });
    assert.equal(patched.status, 200);

    const gone = await app.request(`/v1/events/${uid}`, {
      method: 'DELETE',
      headers: auth(),
    });
    assert.equal(gone.status, 204);
  });

  it('upserts by uid via PUT', async () => {
    const first = await app.request('/v1/events/standup', {
      method: 'PUT',
      headers: auth({ 'content-type': 'application/json' }),
      body: JSON.stringify({
        summary: 'Standup',
        start: '2026-08-14T09:30:00-07:00',
      }),
    });
    assert.equal(first.status, 201);
    const again = await app.request('/v1/events/standup', {
      method: 'PUT',
      headers: auth({ 'content-type': 'application/json' }),
      body: JSON.stringify({
        summary: 'Standup moved',
        start: '2026-08-14T10:00:00-07:00',
      }),
    });
    assert.equal(again.status, 200);
    const body: unknown = await again.json();
    assert.ok(body && typeof body === 'object' && 'sequence' in body);
    assert.equal(body.sequence, 1);
  });
});
