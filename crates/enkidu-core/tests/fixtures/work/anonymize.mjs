// Turns answers recorded from the real GitHub and Jira (read-only) into test fixtures: every key, type, enum and
// structure stays exactly as the services sent it; people, texts, paths, branches' words, URLs and every id that
// could be looked up (GitHub's node ids, commit SHAs, the ids of comments and CI runs, paging cursors, the ids inside
// a ticket's document) are replaced with stable fakes (the same person, commit or comment is the same fake
// everywhere). Ticket keys get the sample projects' keys; ticket and PR numbers are made up from the salt, and a CI
// job's own name becomes "job-3". Usage:
//   ANON_ORG=<github org> ANON_REPO=<repo name> ANON_COMPANY=<company word> ANON_SALT=<any private words> \
//   ANON_KEYS=<real key>=<sample key>[,<real key>=<sample key>…] \
//   node tests/fixtures/work/anonymize.mjs <recorded.json> <fixture.json> <me-github-login> <me-jira-account-id>
// The real names come in from outside so that nothing here names them. The salt makes the fakes: the same salt gives
// the same fake for the same real id in every recording, and without it a fake can't be traced back.
// The recordings themselves are never committed.
//
// A fixture that was made before the ids were replaced is brought up to date in place with:
//   ANON_SALT=… ANON_KEYS=… node tests/fixtures/work/anonymize.mjs --again <fixture.json>…
import { createHash } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";

const again = process.argv[2] === "--again";
const [, , input, output, meLogin, meAccount] = process.argv;
const need = (k) => process.env[k] || (console.error(`set ${k}`), process.exit(2));
const SALT = need("ANON_SALT");
// a real ticket project's key → the sample project's key ("ABC=SH,XYZ=OPS")
const KEYS = new Map(need("ANON_KEYS").split(",").map((pair) => pair.split("=").map((s) => s.trim())));

// ---- what could be looked up, made up -----------------------------------------------------------------------------

const hash = (kind, real) => createHash("sha256").update(`${SALT}:${kind}:${real}`).digest("hex");
/** A made-up number with as many digits as the real one. */
const digits = (kind, real) => {
  const n = BigInt(real.length);
  return String(10n ** (n - 1n) + (BigInt(`0x${hash(kind, real).slice(0, 16)}`) % (9n * 10n ** (n - 1n))));
};
/** A real ticket's or PR's number as a made-up one of as many digits: from the salt, so nothing turns it back. */
const madeUpNumber = (kind, n) => (n.length >= 3 && n.length <= 6 ? digits(kind, n) : n);
/** A ticket key of a real project as the sample project's, with a made-up number: `ABC-12345` → `SH-21654`. Other
 * keys stay. */
const sampleKeys = (text) =>
  text
    .replace(/\b([A-Z][A-Z0-9]+)-(\d+)\b/g, (m, key, n) => (KEYS.has(key) ? `${KEYS.get(key)}-${madeUpNumber("ticket", n)}` : m))
    // the project's key alone: as a whole value ("ABC"), and in a name like "Shop Main (ABC)"
    .replace(/"([A-Z][A-Z0-9]+)"/g, (m, key) => (KEYS.has(key) ? `"${KEYS.get(key)}"` : m))
    .replace(/\(([A-Z][A-Z0-9]+)\)"/g, (m, key) => (KEYS.has(key) ? `(${KEYS.get(key)})"` : m));

/** The last pass over a fixture's text: every id that points at the real services becomes a fake made from it. */
function publicSafe(text) {
  return (
    sampleKeys(text)
      // GitHub's node ids hold the repository's and the object's numbers
      .replace(/\b([A-Z]{1,8})_k[A-Za-z0-9_-]{8,}/g, (m, kind) => `${kind}_${hash("node", m).slice(0, 18)}`)
      // commit SHAs
      .replace(/\b[0-9a-f]{40}\b/g, (m) => hash("sha", m).slice(0, 40))
      // the ids of review comments, comments and CI runs, in links
      .replace(/(discussion_r|issuecomment-|pullrequestreview-)(\d+)/g, (m, what, id) => what + digits("comment", id))
      .replace(
        /(\/actions\/runs\/)(\d+)(\/job\/)(\d+)/g,
        (m, a, run, b, job) => a + digits("run", run) + b + digits("job", job),
      )
      .replace(/\?scope=[A-Za-z0-9_]+/g, "?scope=fixture")
      // GitHub's paging cursors hold a time and an id
      .replace(/"Y3Vyc29yOnYy[A-Za-z0-9+/_=-]+"/g, (m) =>
        JSON.stringify(Buffer.from(`cursor:fixture:${hash("cursor", m).slice(0, 12)}`).toString("base64")),
      )
      // the ids of the blocks inside a ticket's document
      .replace(/\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b/g, (m) => {
        const x = hash("uuid", m);
        return `${x.slice(0, 8)}-${x.slice(8, 12)}-4${x.slice(13, 16)}-8${x.slice(17, 20)}-${x.slice(20, 32)}`;
      })
  );
}

/** A CI job's name says how a company's pipeline is built: it becomes "job-3" (the same name is the same job). The
 * checks of public services keep their names. */
const jobs = new Map();
const checkName = (n) => {
  if (/^(SonarCloud|CodeRabbit)\b/.test(n) || /^job-\d+$/.test(n)) return n;
  if (!jobs.has(n)) jobs.set(n, `job-${jobs.size + 1}`);
  return jobs.get(n);
};

/** What a profile and a ticket's files say about people: a neutral zone and title, file names without a date. */
function tidy(node) {
  if (Array.isArray(node)) return node.map(tidy);
  if (!node || typeof node !== "object") return node;
  const out = {};
  for (const [k, v] of Object.entries(node)) {
    if (typeof v !== "string") out[k] = tidy(v);
    else if (k === "timeZone" || k === "zoneinfo") out[k] = "Etc/UTC";
    else if (k === "jobTitle") out[k] = "Developer";
    else if (k === "localId" && /^[0-9a-f]{12}$/.test(v)) out[k] = hash("local", v).slice(0, 12);
    else if ((k === "alt" || k === "filename") && /\d{6,}|\d{4}-\d{2}-\d{2}/.test(v))
      out[k] = `image-${hash("file", v).slice(0, 6)}${v.includes(".") ? v.slice(v.lastIndexOf(".")) : ""}`;
    else if (k === "name" && node.__typename === "CheckRun") out[k] = checkName(v);
    else if (k === "context" && node.__typename === "StatusContext") out[k] = checkName(v);
    else out[k] = v;
  }
  return out;
}

if (again) {
  for (const file of process.argv.slice(3)) {
    const before = readFileSync(file, "utf8");
    const [was, now] = [JSON.parse(before), tidy(JSON.parse(before))];
    // the file's own writing stays where nothing in it changed, so only what changed shows in the diff
    const indent = /^\{\n( +)/.exec(before)?.[1].length ?? 1;
    const base = JSON.stringify(was) === JSON.stringify(now) ? before : `${JSON.stringify(now, null, indent)}\n`;
    const after = publicSafe(base);
    if (after !== before) writeFileSync(file, after);
    console.log(`${file}: ${after === before ? "as it was" : "brought up to date"}`);
  }
  process.exit(0);
}

// ---- a new recording ----------------------------------------------------------------------------------------------

const ORG = need("ANON_ORG");
const REPO = need("ANON_REPO");
const COMPANY = need("ANON_COMPANY");
const esc = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const orgRepo = new RegExp(`${esc(ORG)}/${esc(REPO)}`, "g");
const orgSlash = new RegExp(`${esc(ORG)}/`, "gi");
const company = new RegExp(esc(COMPANY), "gi");
const site = new RegExp(`${esc(COMPANY)}\\.atlassian\\.net`, "gi");
const data = JSON.parse(readFileSync(input, "utf8"));

const people = new Map(); // a real login, account id or display name → its fake
function person(real, kind) {
  if (real === meLogin || real === meAccount) return kind === "account" ? "acc-me" : "me-dev";
  const key = `${kind}:${real}`;
  if (!people.has(key)) people.set(key, `${kind === "account" ? "acc" : "person"}-${[...people.keys()].filter((k) => k.startsWith(`${kind}:`)).length + 1}`);
  return people.get(key);
}
// display names map through their account id when we know it, so a name and its id stay one person
const nameOf = new Map();

let texts = 0;
let links = 0;
/** A PR's description keeps its Markdown shape (headings, lists, quotes, code fences, tables): every line keeps its
 * marker, and its words become a fake text. */
const fakeMarkdown = (s) =>
  s
    .split("\n")
    .map((line) => {
      if (/^\s*```/.test(line)) return line.replace(/^(\s*```[\w-]*).*$/, "$1");
      const m = /^(\s*(?:#{1,6}\s+|[-*+]\s+(?:\[[ xX]\]\s+)?|\d+[.)]\s+|>\s?|\|)?)(.*)$/.exec(line);
      const [, marker, rest] = m;
      return rest.trim() ? `${marker}${fakeText(rest)}` : line;
    })
    .join("\n");
const fakeText = (s) => `Text ${++texts} (${s.length} chars)${/\?/.test(s) ? " with a question?" : ""}`;
const KEY = /\b([A-Z][A-Z0-9]+-\d+)\b/g;
function keepKeys(s, rest) {
  const keys = [...s.matchAll(KEY)].map((m) => m[1]);
  return keys.length ? `${keys.join(" ")} ${rest}` : rest;
}
function fakeUrl(s) {
  return s
    .replace(orgRepo, "example-org/shop")
    // any other repository of the organisation: its name says what the company builds
    .replace(new RegExp(`${esc(ORG)}/[A-Za-z0-9._-]+`, "gi"), "example-org/other")
    .replace(orgSlash, "example-org/")
    .replace(/https?:\/\/[a-z0-9-]*\.?slack\.com[^\s"]*/gi, "https://chat.example.com/message")
    .replace(site, "example.atlassian.net")
    .replace(/\/ex\/jira\/[0-9a-f-]{36}/g, "/ex/jira/cloud-0")
    .replace(/accountId=[^&"]+/g, "accountId=acc")
    .replace(/\/people\/[^/?#"\s]+/g, "/people/person")
    .replace(/identity::user\/[^/?#"\s]+/g, "identity::user/acc")
    .replace(/https:\/\/sonarcloud\.io[^"]*/g, "https://ci.example.com/sonar")
    // a PR's number in a link
    .replace(/\/pull\/(\d+)/g, (m, n) => `/pull/${madeUpNumber("pr", n)}`)
    // any other service a check or a link points at
    .replace(
      /https:\/\/(?!(?:[a-z0-9.-]*\.)?(?:github\.com|githubusercontent\.com|example\.com|atlassian\.net|atlassian\.com)\b)[^\s"]+/gi,
      "https://ci.example.com/check",
    )
    .replace(company, "example")
    // Jira account ids, wherever they still are: "<digits>:<uuid>" and 24-hex ones
    .replace(/\b\d{6}:[0-9a-f-]{36}\b/g, "acc")
    .replace(/\b[0-9a-f]{24}\b/g, "acc");
}
let paths = 0;
const pathMap = new Map();
function fakePath(p) {
  if (!pathMap.has(p)) {
    const ext = p.includes(".") ? p.slice(p.lastIndexOf(".")) : "";
    pathMap.set(p, `src/module_${++paths}${ext}`);
  }
  return pathMap.get(p);
}
function fakeBranch(b) {
  const m = b.match(/^([a-z]+\/)?([A-Z][A-Z0-9]+-\d+)?/);
  const prefix = m?.[1] ?? "";
  const key = m?.[2] ? `${m[2]}-` : "";
  return `${prefix}${key}change-${[...b].reduce((n, c) => (n * 31 + c.charCodeAt(0)) % 997, 7)}`;
}

function walk(node, parentKey) {
  if (Array.isArray(node)) return node.map((x) => walk(x, parentKey));
  if (!node || typeof node !== "object") return node;
  // an @mention in Jira's document format: the person it points at, and its "@Name" text, become the same fake
  if (node.type === "mention" && node.attrs && typeof node.attrs.id === "string") {
    const id = person(node.attrs.id, "account");
    return { ...node, attrs: { ...node.attrs, id, text: `@${id === "acc-me" ? "Me Dev" : id.replace(/^acc-/, "Person ")}` } };
  }
  // a link in a ticket's description can point anywhere (shared documents, dashboards): it becomes a fake one
  if ((node.type === "link" || node.type === "inlineCard" || node.type === "blockCard") && node.attrs) {
    const attrs = { ...node.attrs };
    for (const k of ["href", "url"]) if (typeof attrs[k] === "string") attrs[k] = `https://example.com/link-${++links}`;
    return { ...node, attrs };
  }
  // an image or file in a description: its ids and names say nothing, but they aren't ours to keep
  if ((node.type === "media" || node.type === "mediaInline") && node.attrs) {
    return { ...node, attrs: { ...node.attrs, id: `media-${++links}`, collection: "", ...(node.attrs.alt ? { alt: `file-${links}` } : {}) } };
  }
  const out = {};
  for (const [k, v] of Object.entries(node)) {
    if (typeof v !== "string") {
      // a PR's (or an issue's) number
      out[k] = k === "number" && Number.isInteger(v) ? Number(madeUpNumber("pr", String(v))) : walk(v, k);
      continue;
    }
    if (k === "login") out[k] = node.__typename === "Bot" ? v : person(v, "login");
    else if (k === "accountId") out[k] = person(v, "account");
    else if (k === "displayName") {
      const id = node.accountId ? person(node.accountId, "account") : person(v, "login");
      nameOf.set(v, id);
      out[k] = id === "acc-me" ? "Me Dev" : id.replace(/^acc-/, "Person ");
    } else if ((k === "name" || k === "nickname") && typeof node.accountId === "string") {
      const id = person(node.accountId, "account");
      out[k] = id === "acc-me" ? "Me Dev" : id.replace(/^acc-/, "Person ");
    } else if (k === "emailAddress" || k === "email") out[k] = "someone@example.com";
    else if (k === "picture") out[k] = "https://example.com/avatar.png";
    else if (k === "body" && "changedFiles" in node) out[k] = fakeMarkdown(v); // a PR's own description
    else if (k === "body" || k === "summary" || k === "description") out[k] = fakeText(v);
    else if (k === "title" || k === "messageHeadline") out[k] = keepKeys(v, fakeText(v));
    else if (k === "text") out[k] = node.type === "mention" ? `@${person(node.id ?? v, "account")}` : fakeText(v);
    else if (k === "path") out[k] = fakePath(v);
    else if (k === "baseRefName" && /^(development|develop|master|main|trunk)$/.test(v)) out[k] = v;
    // a release branch stays one, under a made-up number
    else if (k === "baseRefName" && /^release/.test(v)) out[k] = `release/${1 + (parseInt(hash("release", v).slice(0, 4), 16) % 9)}.${parseInt(hash("release", v).slice(4, 8), 16) % 10}`;
    else if (k === "headRefName" || k === "baseRefName") out[k] = fakeBranch(v);
    else if (k === "filename") out[k] = `file-${texts++}${v.includes(".") ? v.slice(v.lastIndexOf(".")) : ""}`;
    else if (/url$|Url$|^self$|^content$|^thumbnail$|^href$|Link$/.test(k) || /^https?:\/\/|ari:cloud/.test(v)) out[k] = fakeUrl(v);
    else if (k === "id" && node.type === "mention") out[k] = person(v, "account");
    else if (k === "input") out[k] = v; // ticket keys asked for
    else out[k] = v;
  }
  // the mention's text follows the person it points at
  if (node.type === "mention" && out.attrs) out.attrs = { ...out.attrs };
  return out;
}

// a last pass over everything: the company's name in any field (site, project names), e-mail links, then every id
const text = publicSafe(
  JSON.stringify(tidy(walk(data)), null, 1)
    .replace(/mailto:[^"\s]+/g, "mailto:someone@example.com")
    .replace(company, "example"),
);
writeFileSync(output, `${text}\n`);
console.log(`${output}: ${people.size} people, ${texts} texts, ${paths} paths replaced`);
