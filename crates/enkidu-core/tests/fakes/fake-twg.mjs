#!/usr/bin/env node
// A fake `twg` (Atlassian's command line) for tests. It answers like the real one, from the answers recorded from
// Jira (tests/fixtures/work), never touches the network, and logs every call so tests can check that only read
// commands are ever run.
//
// The mode is read on every call from FAKE_TWG_MODE_FILE (or FAKE_TWG_MODE), so an end-to-end test can break it and
// fix it while the app runs:
//   ok, one-page (the search fits one page), logged-out, bad-site, network, rate-limit, garbage, missing-field,
//   item-error (one ticket fails), truncated, moved (a ticket's key changed), describe-error (reading a ticket's
//   description fails), slow, board-error (reading a board fails), board-gone (Jira says the board doesn't exist),
//   board-pages (the boards come three to a page)
// FAKE_TWG_STATUS=KEY=Status/category sets one ticket's status (to move it between reads), FAKE_TWG_UPDATED=KEY=time
// says a ticket changed (its `updated`), and FAKE_TWG_GONE=KEY makes Jira say a ticket doesn't exist.
// FAKE_TWG_SPRINT=KEY,KEY (or FAKE_TWG_SPRINT_FILE) names the tickets in the sprint that is running, in place of the
// recorded ones (twg-sprint.json).
//
// Like the real twg, `jira workitem get` with one key answers with the ticket itself in a list (not items), and a
// ticket that doesn't exist fails the whole command (404).
import { appendFileSync, existsSync, readFileSync, writeSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const fixtures = process.env.FAKE_FIXTURES || join(here, "../fixtures/work");
const args = process.argv.slice(2);
const modeFile = process.env.FAKE_TWG_MODE_FILE;
const readMode = () => ((modeFile && existsSync(modeFile) ? readFileSync(modeFile, "utf8") : process.env.FAKE_TWG_MODE) || "ok").trim() || "ok";
const mode = readMode();
const fixture = (name) => JSON.parse(readFileSync(join(fixtures, name), "utf8"));
// Written in full before exiting: `process.exit` right after `stdout.write` cuts a piped answer at 64 KB.
function writeAll(fd, text) {
  const buf = Buffer.from(text);
  let off = 0;
  while (off < buf.length) {
    try {
      off += writeSync(fd, buf, off, buf.length - off);
    } catch (e) {
      if (e.code !== "EAGAIN") throw e;
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 5); // the pipe is full: let the reader catch up
    }
  }
}
const answer = (value, code = 0) => {
  writeAll(1, typeof value === "string" ? value : JSON.stringify(value));
  process.exit(code);
};
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};

// `twg --version`: Setup reads the version from it, whoever is signed in. It asks Jira nothing, so it isn't logged
if (args[0] === "--version") answer("1.4.2\n");

if (process.env.FAKE_TWG_LOG) appendFileSync(process.env.FAKE_TWG_LOG, `${mode} | ${args.join(" ")}\n`);

// the commands that write: never allowed
const writes = ["create", "update", "edit", "delete", "transition", "assign", "add", "remove", "move", "login", "logout"];
if (args.some((a) => writes.includes(a))) answer({ ok: false, error: { code: "FAKE_REFUSED", message: `fake twg: writing is never allowed (${args.join(" ")})` } }, 2);


const failures = {
  "logged-out": () => answer(fixture("twg-logged-out.json"), 77),
  "bad-site": () => answer(fixture("twg-bad-site.json"), 1),
  network: () => answer({ ok: false, error: { code: "TWG_TOOL_EXECUTION_FAILED", kind: "twg_tool_execution_failed", message: "fetch failed: getaddrinfo ENOTFOUND api.atlassian.com" } }, 1),
  "rate-limit": () => answer({ ok: false, error: { code: "TWG_TOOL_EXECUTION_FAILED", statusCode: 429, message: "Too Many Requests" } }, 1),
  garbage: () => answer("Updating twg…\n<<not json>>\n"),
};
failures[mode]?.();
if (mode === "slow") await new Promise((r) => setTimeout(r, 120_000));

const [statusKey, statusValue] = (process.env.FAKE_TWG_STATUS ?? "").split("=");
const [statusName, statusCategory] = (statusValue ?? "").split("/");
const setStatus = (t) => {
  if (statusKey && t.key === statusKey) t.status = { ...t.status, name: statusName, statusCategory: { ...t.status.statusCategory, key: statusCategory || "indeterminate" } };
};

const [updatedKey, updatedAt] = (process.env.FAKE_TWG_UPDATED ?? "").split("=");
const setUpdated = (t) => {
  if (updatedKey && t.key === updatedKey) t.updated = updatedAt;
};

const gone = (process.env.FAKE_TWG_GONE ?? "").split(",").filter(Boolean);
// FAKE_TWG_ASSIGNEE=KEY=accountId/Name: the ticket is someone else's now (so your search no longer finds it)
const [otherKey, otherWho] = (process.env.FAKE_TWG_ASSIGNEE ?? "").split("=");
const [otherId, otherName] = (otherWho ?? "").split("/");
const yours = (t) => !gone.includes(t.key) && t.key !== otherKey;

if (args[0] === "whoami") answer(fixture("twg-whoami.json"));

// Jira's boards: the ones of a Jira project (the choice in the Work page's header), from the recorded answers.
if (args[0] === "jira" && args[1] === "board" && args[2] === "query") {
  const project = opt("--project");
  const file = { SH: "twg-boards.json", OPS: "twg-boards-ops.json" }[project];
  if (!file) answer({ ok: false, error: { code: "TWG_TOOL_EXECUTION_FAILED", kind: "twg_tool_execution_failed", message: `Board search failed: No project could be found with key '${project}'.`, statusCode: 400 } }, 1);
  const v = fixture(file);
  const start = Number(opt("--start-at") ?? 0);
  const size = mode === "board-pages" ? 3 : Number(opt("--max-results") ?? 50);
  const all = v.data.boards;
  const boards = all.slice(start, start + size);
  v.data = { ...v.data, boards, startAt: start, maxResults: size, isLast: start + boards.length >= all.length };
  if (mode === "missing-field" && boards[0]) delete boards[0].type;
  answer(v);
}

// What is on a board: its saved search, its columns and their statuses, from the recorded answers (boards 5, 9, 20).
if (args[0] === "jira" && args[1] === "board" && args[2] === "scope" && args[3] === "query") {
  const id = opt("--id");
  if (mode === "board-error") answer({ ok: false, error: { code: "TWG_TOOL_EXECUTION_FAILED", kind: "twg_tool_execution_failed", statusCode: 500, message: "Board scope query failed: Internal server error" } }, 1);
  if (mode === "board-gone" || !/^\d+$/.test(id ?? "") || !existsSync(join(fixtures, `twg-board-scope-${id}.json`))) answer(fixture("twg-board-gone.json"), 1);
  answer(fixture(`twg-board-scope-${id}.json`));
}

// The tickets on a board: a search that starts with the board's saved search (`filter = N`). Of the tickets asked
// about (`key in (…)`), the ones in a status of the board's columns (`status in (…)`) and, on a scrum board (`sprint
// in openSprints()`), in the sprint that is running. Like Jira, a search that names a key it doesn't know fails whole.
if (args[0] === "jira" && args[1] === "workitem" && args[2] === "query" && /^filter = \d+/.test(opt("--jql") ?? "")) {
  const jql = opt("--jql");
  const list = (name) => (new RegExp(`\\b${name} in \\(([^)]*)\\)`).exec(jql)?.[1] ?? "").split(",").map((x) => x.trim()).filter(Boolean);
  const sprintFile = process.env.FAKE_TWG_SPRINT_FILE;
  const sprint = !/\bsprint in openSprints\(\)/.test(jql)
    ? null
    : sprintFile && existsSync(sprintFile)
      ? readFileSync(sprintFile, "utf8").split(/[\s,]+/).filter(Boolean)
      : process.env.FAKE_TWG_SPRINT
        ? process.env.FAKE_TWG_SPRINT.split(",")
        : fixture("twg-sprint.json");
  const known = new Map();
  for (const t of [1, 2, 3].flatMap((p) => fixture(`twg-query-page${p}.json`).data.issues)) known.set(t.key, t);
  for (const i of fixture("twg-get.json").data.items) if (i.ok && !known.has(i.data.key)) known.set(i.data.key, i.data);
  const asked = list("key");
  const unknown = asked.find((k) => !known.has(k) || gone.includes(k));
  if (unknown) answer({ ok: false, error: { code: "TWG_TOOL_EXECUTION_FAILED", kind: "twg_tool_execution_failed", statusCode: 400, message: `Search failed: An issue with key '${unknown}' does not exist for field 'key'.` } }, 1);
  const statuses = list("status");
  const issues = asked
    .map((k) => structuredClone(known.get(k)))
    .filter((t) => statuses.includes(String(t.status.id)) && (!sprint || sprint.includes(t.key)));
  issues.forEach(setStatus);
  if (mode === "missing-field" && issues[0]) delete issues[0].status.statusCategory;
  answer({ apiVersion: "v2", command: "jira.workitem.query", request: { jql, site: "example.atlassian.net", maxResults: Number(opt("--limit") ?? 50) }, data: { issues } });
}

if (args[0] === "jira" && args[1] === "workitem" && args[2] === "query") {
  if (mode === "one-page") {
    const v = fixture("twg-tickets.json");
    v.data.issues = v.data.issues.filter(yours);
    v.data.issues.forEach(setStatus);
    v.data.issues.forEach(setUpdated);
    answer(v);
  }
  const page = { undefined: 1, "cursor-1": 2, "cursor-2": 3 }[opt("--after")];
  if (!page) answer({ ok: false, error: { code: "BAD_CURSOR", message: `unknown cursor ${opt("--after")}` } }, 1);
  const v = fixture(`twg-query-page${page}.json`);
  v.data.issues = v.data.issues.filter(yours);
  if (mode === "missing-field" && page === 1) delete v.data.issues[0].status.statusCategory;
  v.data.issues.forEach(setStatus);
  v.data.issues.forEach(setUpdated);
  answer(v);
}

// A ticket's description, type and priority (read for new and changed tickets), from the recorded answers.
const asked = () => args.slice(3).filter((a) => /^[A-Z][A-Z0-9]+-\d+$/.test(a));
if (args[0] === "jira" && args[1] === "workitem" && args[2] === "get" && opt("--fields") === "description,issuetype,priority") {
  const keys = asked();
  const known = fixture("twg-describe.json").data.items.filter((i) => i.ok);
  const find = (key) => (gone.includes(key) ? undefined : known.find((i) => i.input === key));
  if (keys.length === 1) {
    const found = find(keys[0]);
    if (!found) answer(fixture("twg-get-single-gone.json"), 1);
    if (mode === "describe-error") answer({ ok: false, error: { code: "TWG_TOOL_EXECUTION_FAILED", statusCode: 500, message: "Internal server error" } }, 1);
    const v = fixture("twg-describe-single.json");
    v.request.issueIdOrKey = keys[0];
    v.data = [found.data];
    answer(v);
  }
  const items = keys.map((key, n) => {
    const found = find(key);
    if (!found) return { input: key, ok: false, error: { message: "Issue does not exist or you do not have permission to see it.", status: 404 } };
    if (mode === "describe-error" && n === 0) return { input: key, ok: false, error: { message: "Internal server error", status: 500 } };
    return structuredClone(found);
  });
  const failed = items.filter((i) => !i.ok).length;
  const summary = { requested: keys.length, returned: items.length, succeeded: keys.length - failed, failed, omitted: 0, truncated: 0 };
  answer({ apiVersion: "v2", command: "jira.workitem.get", request: { issueIdOrKey: keys, site: "example.atlassian.net", fields: ["description", "issuetype", "priority"] }, data: { items, summary } }, failed ? 3 : 0);
}

// The commands the live check (Check connections, enkidu doctor) asks directly, answered from the same recorded tickets.
if (args[0] === "jira" && args[1] === "workitem" && args[2] === "bulk-get") {
  const keys = args.slice(3).filter((a) => /^[A-Z][A-Z0-9]+-\d+$/.test(a));
  // the tickets read whole, and the ones only the search finds (a ticket in the backlog)
  const known = [
    ...fixture("twg-get.json").data.items.filter((i) => i.ok).map((i) => i.data),
    ...[1, 2, 3].flatMap((p) => fixture(`twg-query-page${p}.json`).data.issues),
  ];
  const described = fixture("twg-describe.json").data.items.filter((i) => i.ok);
  const fields = (opt("--fields") ?? "").split(",");
  const items = keys.map((key) => {
    const found = known.find((t) => t.key === key);
    if (!found || gone.includes(key)) return { input: key, ok: false, error: { message: "Issue does not exist", status: 404 } };
    const d = structuredClone(found);
    setStatus(d);
    if (d.key === otherKey) d.assignee = { ...d.assignee, accountId: otherId, displayName: otherName };
    const data = { key: d.key, status: d.status, assignee: d.assignee, url: d.url };
    const more = described.find((i) => i.input === key)?.data;
    for (const f of ["issuetype", "priority", "description"]) if (fields.includes(f)) data[f] = more?.[f] ?? null;
    return { input: key, ok: true, data };
  });
  answer({ apiVersion: "v2", command: "jira.workitem.bulk-get", data: { items, summary: { requested: keys.length, returned: items.length } } });
}
if (args[0] === "jira" && args[1] === "workitem" && args[2] === "comment" && args[3] === "query") {
  const key = opt("--issue-id");
  const found = fixture("twg-get.json").data.items.find((i) => i.ok && i.input === key);
  answer({ apiVersion: "v2", command: "jira.workitem.comment.query", data: found?.data.comments ?? [], pageInfo: { hasNextPage: false, nextCursor: null } });
}

if (args[0] === "jira" && args[1] === "workitem" && args[2] === "get") {
  const keys = args.slice(3).filter((a) => /^[A-Z][A-Z0-9]+-\d+$/.test(a));
  const known = fixture("twg-get.json").data.items.filter((i) => i.ok);
  const items = keys.map((key, n) => {
    const found = known.find((i) => i.input === key);
    if (!found || gone.includes(key)) return { input: key, ok: false, error: { message: "Issue does not exist or you do not have permission to see it.", status: 404 } };
    if (mode === "item-error" && n === 0) return { input: key, ok: false, error: { message: "Internal server error", status: 500 } };
    const item = structuredClone(found);
    setStatus(item.data);
    setUpdated(item.data);
    if (item.data.key === otherKey) item.data.assignee = { ...item.data.assignee, accountId: otherId, displayName: otherName };
    if (mode === "moved" && n === 0) item.data.key = `XY-${item.data.key.split("-")[1]}`;
    return item;
  });
  // one key: the ticket itself in a list, or the whole command fails
  if (keys.length === 1) {
    const [item] = items;
    if (!item.ok) {
      const status = item.error.status;
      answer(status === 404 ? fixture("twg-get-single-gone.json") : { ok: false, error: { code: "TWG_TOOL_EXECUTION_FAILED", statusCode: status, message: item.error.message } }, 1);
    }
    const v = fixture("twg-get-single.json");
    v.request.issueIdOrKey = keys[0];
    v.data = [item.data];
    answer(v);
  }
  const failed = items.filter((i) => !i.ok).length;
  const summary = { requested: keys.length, returned: items.length, succeeded: keys.length - failed, failed, omitted: 0, truncated: mode === "truncated" ? 2 : 0 };
  answer({ apiVersion: "v2", command: "jira.workitem.get", request: { issueIdOrKey: keys }, data: { items, summary } }, failed ? 3 : 0);
}

answer({ ok: false, error: { code: "FAKE_UNKNOWN", message: `fake twg: no answer for: ${args.join(" ")}` } }, 1);
