import { describe, it, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { openDb } from './db.ts';
import { createApp } from './server.ts';

const FEED = 'feed-secret';
const KEY = 'agent-secret';
const BASE = 'http://example.test';

let app: ReturnType<typeof createApp>;

beforeEach(() => {
  const dir = mkdtempSync(join(tmpdir(), 'almanac-srv-'));
  app = createApp({
    db: openDb(join(dir, 'calendar.db')),
    publicBase: BASE,
    home: { feedToken: FEED, agentKey: KEY, name: 'My Calendar' },
  });
});

function auth(extra?: Record<string, string>): Record<string, string> {
  return { authorization: `Bearer ${KEY}`, ...extra };
}

describe('landing', () => {
  it('returns json for agents', async () => {
    const res = await app.request('/', { headers: { accept: 'application/json' } });
    assert.equal(res.status, 200);
    const body: unknown = await res.json();
    assert.ok(body && typeof body === 'object' && 'create' in body);
  });

  it('returns html for browsers', async () => {
    const res = await app.request('/', { headers: { accept: 'text/html' } });
    assert.equal(res.status, 200);
    const html = await res.text();
    assert.match(html, /Create Calendar/);
    assert.match(html, /mascot\.webp/);
  });
});

describe('provision', () => {
  it('creates a calendar and returns subscribe + key', async () => {
    const res = await app.request('/calendars', {
      method: 'POST',
      headers: { 'content-type': 'application/json', accept: 'application/json' },
      body: JSON.stringify({ name: 'Giants' }),
    });
    assert.equal(res.status, 201);
    const body: unknown = await res.json();
    assert.ok(body && typeof body === 'object');
    assert.ok('id' in body && typeof body.id === 'string');
    assert.ok('subscribe' in body && typeof body.subscribe === 'string');
    assert.ok('key' in body && typeof body.key === 'string');
    assert.match(body.subscribe, /^http:\/\/example\.test\/feed\/.+\.ics$/);
  });

  it('isolates two calendars', async () => {
    const a = await app.request('/calendars', {
      method: 'POST',
      headers: { 'content-type': 'application/json', accept: 'application/json' },
      body: '{}',
    });
    const b = await app.request('/calendars', {
      method: 'POST',
      headers: { 'content-type': 'application/json', accept: 'application/json' },
      body: '{}',
    });
    const aBody: unknown = await a.json();
    const bBody: unknown = await b.json();
    assert.ok(aBody && typeof aBody === 'object' && 'id' in aBody && 'key' in aBody);
    assert.ok(bBody && typeof bBody === 'object' && 'id' in bBody && 'key' in bBody);
    const aId = aBody.id;
    const aKey = aBody.key;
    const bId = bBody.id;
    const bKey = bBody.key;
    assert.equal(typeof aId, 'string');
    assert.equal(typeof aKey, 'string');
    assert.equal(typeof bId, 'string');
    assert.equal(typeof bKey, 'string');

    const put = await app.request(`/v1/c/${aId}/events/only-a`, {
      method: 'PUT',
      headers: {
        authorization: `Bearer ${aKey}`,
        'content-type': 'application/json',
      },
      body: JSON.stringify({ summary: 'Only A', start: '2026-08-16T12:00:00Z' }),
    });
    assert.equal(put.status, 201);

    const other = await app.request(`/v1/c/${bId}/events`, {
      headers: { authorization: `Bearer ${bKey}` },
    });
    const listed: unknown = await other.json();
    assert.ok(listed && typeof listed === 'object' && 'events' in listed);
    assert.ok(Array.isArray(listed.events));
    assert.equal(listed.events.length, 0);

    const steal = await app.request(`/v1/c/${aId}/events`, {
      headers: { authorization: `Bearer ${bKey}` },
    });
    assert.equal(steal.status, 401);
  });
});

describe('feed', () => {
  it('404s a wrong token', async () => {
    const res = await app.request('/feed/nope.ics');
    assert.equal(res.status, 404);
  });

  it('returns text/calendar for the home token', async () => {
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
