/** RFC 5545 text escaping for SUMMARY / DESCRIPTION / LOCATION. */
export function escapeText(value: string): string {
  return value
    .replaceAll('\\', '\\\\')
    .replaceAll('\r\n', '\n')
    .replaceAll('\r', '\n')
    .replaceAll('\n', '\\n')
    .replaceAll(';', '\\;')
    .replaceAll(',', '\\,');
}

/** Fold a content line at 75 octets. Continuations start with a space. */
export function foldLine(line: string): string {
  const enc = new TextEncoder();
  const dec = new TextDecoder();
  const bytes = enc.encode(line);
  if (bytes.length <= 75) return line;
  const parts: string[] = [];
  let i = 0;
  let limit = 75;
  while (i < bytes.length) {
    let end = Math.min(i + limit, bytes.length);
    while (end > i && (bytes[end] & 0xc0) === 0x80) end -= 1;
    if (end === i) end = Math.min(i + limit, bytes.length);
    parts.push(dec.decode(bytes.subarray(i, end)));
    i = end;
    limit = 74;
  }
  return [parts[0], ...parts.slice(1).map((p) => ` ${p}`)].join('\r\n');
}

/** Instant → `YYYYMMDDTHHMMSSZ`. */
export function toIcsUtc(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) {
    throw new Error(`invalid date: ${iso}`);
  }
  return d.toISOString().replace(/[-:]/g, '').replace(/\.\d{3}/, '');
}

/** Calendar date `YYYY-MM-DD` → `YYYYMMDD`. */
export function toIcsDate(ymd: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(ymd);
  if (!m) throw new Error(`invalid all-day date: ${ymd}`);
  return `${m[1]}${m[2]}${m[3]}`;
}

/** Exclusive next DATE for all-day DTEND. */
export function nextDate(ymd: string): string {
  const d = new Date(`${ymd}T00:00:00Z`);
  if (Number.isNaN(d.getTime())) throw new Error(`invalid all-day date: ${ymd}`);
  d.setUTCDate(d.getUTCDate() + 1);
  const y = String(d.getUTCFullYear()).padStart(4, '0');
  const mo = String(d.getUTCMonth() + 1).padStart(2, '0');
  const day = String(d.getUTCDate()).padStart(2, '0');
  return `${y}-${mo}-${day}`;
}

export type IcsEvent = {
  uid: string;
  summary: string;
  description: string;
  location: string;
  dtstart: string;
  dtend: string;
  allDay: boolean;
  transparent: boolean;
  sequence: number;
  updatedAt: string;
};

function stamp(iso: string): string {
  try {
    return toIcsUtc(iso);
  } catch {
    return toIcsUtc(new Date().toISOString());
  }
}

function vevent(ev: IcsEvent): string {
  const lines = [
    'BEGIN:VEVENT',
    `UID:${ev.uid}`,
    `DTSTAMP:${stamp(ev.updatedAt)}`,
    `LAST-MODIFIED:${stamp(ev.updatedAt)}`,
    `SEQUENCE:${ev.sequence}`,
    `SUMMARY:${escapeText(ev.summary)}`,
  ];
  if (ev.allDay) {
    lines.push(`DTSTART;VALUE=DATE:${toIcsDate(ev.dtstart)}`);
    lines.push(`DTEND;VALUE=DATE:${toIcsDate(ev.dtend)}`);
  } else {
    lines.push(`DTSTART:${toIcsUtc(ev.dtstart)}`);
    lines.push(`DTEND:${toIcsUtc(ev.dtend)}`);
  }
  if (ev.location) lines.push(`LOCATION:${escapeText(ev.location)}`);
  if (ev.description) lines.push(`DESCRIPTION:${escapeText(ev.description)}`);
  lines.push(ev.transparent ? 'TRANSP:TRANSPARENT' : 'TRANSP:OPAQUE');
  lines.push('END:VEVENT');
  return lines.map(foldLine).join('\r\n');
}

/** Build a PUBLISH calendar. Stable UIDs; SEQUENCE tracks edits. */
export function renderCalendar(calName: string, events: IcsEvent[]): string {
  const head = [
    'BEGIN:VCALENDAR',
    'VERSION:2.0',
    'PRODID:-//Steve//almanac//EN',
    'CALSCALE:GREGORIAN',
    'METHOD:PUBLISH',
    `X-WR-CALNAME:${escapeText(calName)}`,
  ].map(foldLine);
  const body = events.map(vevent);
  return [...head, ...body, 'END:VCALENDAR'].join('\r\n') + '\r\n';
}
