import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  escapeText,
  foldLine,
  nextDate,
  renderCalendar,
  toIcsDate,
  toIcsUtc,
} from './ics.ts';

describe('escapeText', () => {
  it('escapes ics specials', () => {
    assert.equal(escapeText('a;b,c\\d\ne'), 'a\\;b\\,c\\\\d\\ne');
  });
});

describe('foldLine', () => {
  it('leaves short lines alone', () => {
    assert.equal(foldLine('SUMMARY:Giants'), 'SUMMARY:Giants');
  });

  it('folds long lines at 75 octets', () => {
    const line = `DESCRIPTION:${'x'.repeat(80)}`;
    const folded = foldLine(line);
    assert.match(folded, /\r\n /);
    const first = folded.split('\r\n')[0] ?? '';
    assert.ok(Buffer.byteLength(first) <= 75);
  });
});

describe('dates', () => {
  it('formats utc instants', () => {
    assert.equal(toIcsUtc('2026-08-16T20:05:00.000Z'), '20260816T200500Z');
  });

  it('formats all-day dates', () => {
    assert.equal(toIcsDate('2026-08-16'), '20260816');
    assert.equal(nextDate('2026-08-16'), '2026-08-17');
  });
});

describe('renderCalendar', () => {
  it('emits a subscribeable publish calendar', () => {
    const ics = renderCalendar('My Calendar', [
      {
        uid: 'game-1@my-calendar',
        summary: 'Rockies @ Giants',
        description: 'NBCS BA',
        location: 'Oracle Park',
        dtstart: '2026-08-16T20:05:00.000Z',
        dtend: '2026-08-16T23:05:00.000Z',
        allDay: false,
        transparent: true,
        sequence: 0,
        updatedAt: '2026-08-13T17:00:00.000Z',
      },
    ]);
    assert.match(ics, /^BEGIN:VCALENDAR/);
    assert.match(ics, /X-WR-CALNAME:My Calendar/);
    assert.match(ics, /UID:game-1@my-calendar/);
    assert.match(ics, /DTSTART:20260816T200500Z/);
    assert.match(ics, /TRANSP:TRANSPARENT/);
    assert.match(ics, /END:VCALENDAR\r\n$/);
  });

  it('uses VALUE=DATE for all-day events', () => {
    const ics = renderCalendar('My Calendar', [
      {
        uid: 'off-1',
        summary: 'Out',
        description: '',
        location: '',
        dtstart: '2026-08-20',
        dtend: '2026-08-21',
        allDay: true,
        transparent: true,
        sequence: 1,
        updatedAt: '2026-08-13T17:00:00.000Z',
      },
    ]);
    assert.match(ics, /DTSTART;VALUE=DATE:20260820/);
    assert.match(ics, /DTEND;VALUE=DATE:20260821/);
  });
});
