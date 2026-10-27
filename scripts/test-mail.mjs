// Exercise delivery confirmation and ambiguous-send recovery against a loopback transport.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { createServer } from 'node:http';
import { randomUUID } from 'node:crypto';
import { resolve } from 'node:path';

const databaseURL = process.env.TEST_DATABASE_URL;
assert.ok(databaseURL, 'TEST_DATABASE_URL must identify the isolated product-test database');
const database = new URL(databaseURL).pathname.slice(1);
assert.match(database, /^opportunity_journey_\d+$/);
const sql = query => execFileSync('docker', ['exec', '-i', process.env.TEST_DB_CONTAINER || 'opportunity-db', 'psql', '-U', 'opportunity', '-d', database, '-At', '-v', 'ON_ERROR_STOP=1'], { input: query, encoding: 'utf8' }).trim();
const success = randomUUID(), ambiguous = randomUUID();
const sends = new Map();
const env = { ...process.env, DATABASE_URL: databaseURL, ACCESS_MODE: 'native', APPLICATION_ENV: 'development', MAIL_API_URL: 'http://127.0.0.1:8910', MAIL_API_KEY: 'synthetic', MAIL_FROM: 'test@example.test', MAIL_TO: 'owner@example.test' };
execFileSync(resolve('target/debug/opportunity'), ['migrate'], { env });
const application = sql("select id from applications where opportunity_id='00000000-0000-4000-8000-000000000003' and status='submitted'");
assert.match(application, /^[a-f0-9-]{36}$/);
sql(`insert into notifications(id,owner_id,application_id,kind,dedupe_key,payload) values('${success}',(select owner_id from applications where id='${application}'),'${application}','submitted','${success}','{}'),('${ambiguous}',(select owner_id from applications where id='${application}'),'${application}','submitted','${ambiguous}','{}');`);
const server = createServer(async (req, res) => {
  if (req.method === 'POST') {
    let raw = ''; for await (const c of req) raw += c;
    assert.deepEqual(JSON.parse(raw).to, ['owner@example.test']);
    const key = req.headers['idempotency-key'];
    assert.ok([success, ambiguous].includes(key));
    sends.set(key, (sends.get(key) || 0) + 1);
    if (key === ambiguous) { req.socket.destroy(); return; }
    res.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({ id: key }));
  } else {
    assert.equal(req.url, '/emails/' + success);
    res.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify({ last_event: 'delivered' }));
  }
});
await new Promise(r => server.listen(8910, '127.0.0.1', r));
const app = spawn(resolve('target/debug/opportunity'), ['serve', '--port', '8899'], { env, stdio: ['ignore', 'ignore', 'inherit'] });
async function until(query, value) {
  for (let i = 0; i < 60; i++) { if (sql(query) === value) return; await new Promise(r => setTimeout(r, 1000)); }
  throw Error('Mail state did not reach ' + value);
}
try {
  await until(`select delivery_state from notifications where id='${success}'`, 'accepted');
  assert.equal(sql(`select delivered_at is null from notifications where id='${success}'`), 't');
  await until(`select first_attempt_at is not null and delivery_error is not null from notifications where id='${ambiguous}'`, 't');
  sql(`update notifications set first_attempt_at=now()-interval '24 hours' where id='${ambiguous}'; update notifications set last_attempt_at=now()-interval '2 minutes' where id in ('${success}','${ambiguous}');`);
  await until(`select delivery_state from notifications where id='${success}'`, 'delivered');
  await until(`select delivery_state from notifications where id='${ambiguous}'`, 'failed');
  assert.equal(sends.get(success), 1);
  assert.equal(sends.get(ambiguous), 1);
  console.log('PASS acceptance is not delivery; provider confirmation required; ambiguous send is not retried outside the idempotency window; no duplicate emails');
} finally {
  app.kill('SIGTERM');
  server.closeAllConnections(); server.close();
  sql(`delete from notifications where id in ('${success}','${ambiguous}');`);
}
