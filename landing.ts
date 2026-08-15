import type { CalendarRow } from './db.ts';
import { jsonCalendar } from './db.ts';

function esc(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;');
}

export function jsonIndex(base: string): Record<string, unknown> {
  const origin = base.replace(/\/$/, '');
  return {
    name: 'Almanac',
    description: 'Agent-first calendar. POST /calendars. Humans subscribe to the https feed URL.',
    create: `POST ${origin}/calendars`,
    docs: `${origin}/llms.txt`,
  };
}

export function llmsTxt(base: string): string {
  const origin = base.replace(/\/$/, '');
  return [
    '# Almanac',
    '',
    'Agent-first ICS calendar. Agents write events. Humans subscribe to the feed URL.',
    '',
    'Every request must send a proper User-Agent header. Empty or default curl is blocked.',
    '',
    `POST ${origin}/calendars`,
    'Optional JSON body: { "name": "Optional Title" }',
    'Returns id, subscribe URL, write URL, and key. Store the key. It is shown once.',
    '',
    'Write (idempotent):',
    `PUT ${origin}/v1/c/{id}/events/{uid}`,
    'Authorization: Bearer {key}',
    'Content-Type: application/json',
    '{ "summary": "Dentist", "start": "2026-08-18T09:00:00-07:00", "end": "2026-08-18T09:45:00-07:00" }',
    '',
    'List: GET /v1/c/{id}/events',
    'Patch: PATCH /v1/c/{id}/events/{uid}',
    'Delete: DELETE /v1/c/{id}/events/{uid}',
    '',
    'Subscribe: GET {subscribe} as text/calendar. No bearer. Use https, not webcal.',
    'Calendars poll. Refresh if a new event is missing.',
    '',
  ].join('\n');
}

const DESCRIPTION =
  'An agent calendar. Agents write events. You subscribe in the app you already have.';

function page(title: string, body: string, origin = ''): string {
  const image = origin ? `${origin}/og.png` : '/og.png';
  const url = origin || '';
  return `<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>${esc(title)}</title>
  <meta name="description" content="${esc(DESCRIPTION)}">
  <meta property="og:type" content="website">
  <meta property="og:title" content="${esc(title)}">
  <meta property="og:description" content="${esc(DESCRIPTION)}">
  <meta property="og:image" content="${esc(image)}">
  <meta property="og:image:width" content="1200">
  <meta property="og:image:height" content="630">
  <meta property="og:image:alt" content="Almanac. An Agent Calendar.">
  ${url ? `<meta property="og:url" content="${esc(url)}">` : ''}
  <meta name="twitter:card" content="summary_large_image">
  <meta name="twitter:title" content="${esc(title)}">
  <meta name="twitter:description" content="${esc(DESCRIPTION)}">
  <meta name="twitter:image" content="${esc(image)}">
  <style>
    :root { color-scheme: dark; }
    body { font: 16px/1.45 ui-sans-serif, system-ui, sans-serif; max-width: 40rem; margin: 3rem auto; padding: 0 1.25rem; color: #e8e8e8; background: #111; }
    .og { display: block; width: 100%; height: auto; margin: 0 0 1.25rem; }
    h1 { font-size: 1.5rem; font-weight: 600; letter-spacing: -0.02em; margin-bottom: 0.25rem; }
    .sub { margin: 0 0 1rem; color: #888; }
    code, pre { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 0.85rem; }
    pre { background: #1c1c1c; padding: 0.9rem 1rem; overflow-x: auto; }
    a { color: #c8c8c8; }
    label { display: block; margin: 1rem 0 0.35rem; font-size: 0.85rem; }
    input { width: 100%; box-sizing: border-box; padding: 0.5rem 0.6rem; background: #1c1c1c; color: inherit; border: 1px solid #333; }
    button { margin-top: 0.9rem; margin-right: 0.5rem; padding: 0.5rem 0.9rem; background: #e8e8e8; color: #111; border: 0; font-weight: 600; }
    button.secondary { background: #1c1c1c; color: #e8e8e8; border: 1px solid #333; }
    .muted { color: #888; }
    #prompt { margin-top: 1.5rem; white-space: pre-wrap; }
  </style>
</head>
<body>
${body}
</body>
</html>
`;
}

export function htmlHome(base: string): string {
  const origin = base.replace(/\/$/, '');
  const prompt = [
    'Almanac is a hosted ICS calendar. You create one over HTTP. I subscribe to the feed in my own calendar app. You write events later when I ask.',
    '',
    `1. GET ${origin}/llms.txt and follow it.`,
    `2. Create a calendar:`,
    `   POST ${origin}/calendars`,
    '   Content-Type: application/json',
    '   Body may be {} or { "name": "optional title" }',
    '3. The JSON response has:',
    '   - id',
    '   - subscribe  (https://…/feed/<token>.ics)',
    '   - write',
    '   - key',
    '4. Store id and key. You need them to add or change events.',
    '5. Give me those four values in plain text.',
    '6. Help me subscribe. The feed is HTTPS ICS. Do not use webcal://.',
    '   Walk me through my calendar of choice:',
    '   - Apple Calendar: File → New Calendar Subscription → paste subscribe',
    '   - Google Calendar: Settings → Add calendar → From URL → paste subscribe',
    '   - Outlook: Add calendar → Subscribe from web → paste subscribe',
    '   - Anything else that accepts an https .ics URL: paste subscribe there',
    '7. Do not add events unless I ask. Setup is create + subscribe only.',
    '',
    `Base: ${origin}`,
  ].join('\n');
  return page('Almanac', `
  <img class="og" src="/og.png" width="1200" height="630" alt="Almanac. An Agent Calendar.">
  <h1>Almanac</h1>
  <p class="sub">An Agent Calendar</p>
  <p>Send your agent here. <a href="/llms.txt">/llms.txt</a></p>
  <pre id="prompt">${esc(prompt)}</pre>
  <button type="button" class="secondary" id="copy">Copy Prompt</button>
  <script>
    document.getElementById('copy').onclick = function () {
      navigator.clipboard.writeText(document.getElementById('prompt').textContent);
    };
  </script>
`, origin);
}

export function htmlCreated(row: CalendarRow, base: string): string {
  const creds = jsonCalendar(row, base);
  const subscribe = String(creds.subscribe);
  const key = String(creds.key);
  const write = String(creds.write);
  return page('Almanac Calendar', `
  <h1>Calendar Ready</h1>
  <p>Save the key. It is not shown again in this form.</p>
  <p><strong>Id</strong><br><code>${esc(row.id)}</code></p>
  <p><strong>Subscribe</strong><br><code>${esc(subscribe)}</code></p>
  <p><strong>Key</strong><br><code>${esc(key)}</code></p>
  <p><strong>Write</strong><br><code>PUT ${esc(write)}/{uid}</code></p>
  <pre>Authorization: Bearer ${esc(key)}</pre>
  <p><a href="/">Back</a></p>
`, base.replace(/\/$/, ''));
}
