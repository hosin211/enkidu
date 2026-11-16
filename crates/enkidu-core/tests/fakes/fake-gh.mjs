#!/usr/bin/env node
// A fake `gh` for tests. It answers like the real one, from the answers recorded from GitHub (tests/fixtures/work),
// never touches the network, and logs every call (without its environment) so tests can check that nothing is ever
// written to GitHub and that no token is ever passed as an argument.
//
// The mode is read on every call from FAKE_GH_MODE_FILE (or FAKE_GH_MODE), so an end-to-end test can break it and
// fix it while the app runs:
//   ok, pages (every list over more than one page), empty (no PRs), logged-out, bad-credentials, no-access,
//   rate-limit, network, garbage, graphql-error, missing-field, slow
import { appendFileSync, existsSync, readFileSync, writeSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const fixtures = process.env.FAKE_FIXTURES || join(here, "../fixtures/work");
const args = process.argv.slice(2);
const modeFile = process.env.FAKE_GH_MODE_FILE;
const mode = ((modeFile && existsSync(modeFile) ? readFileSync(modeFile, "utf8") : process.env.FAKE_GH_MODE) || "ok").trim() || "ok";
const me = process.env.FAKE_GH_LOGIN || "me-dev";
const token = `fake-token-${me}`;

const read = (name) => readFileSync(join(fixtures, name), "utf8");
const fixture = (name) => JSON.parse(read(name));
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
const answer = (value) => {
  writeAll(1, typeof value === "string" ? value : JSON.stringify(value));
  process.exit(0);
};
const fail = (code, text) => {
  writeAll(2, text.endsWith("\n") ? text : `${text}\n`);
  process.exit(code);
};

// `gh --version`, as the real one prints it: Setup reads the version from it, whoever is signed in
if (args[0] === "--version") answer("gh version 2.103.3 (2026-10-26)\nhttps://github.com/cli/cli/releases/tag/v2.103.3\n");

// `gh api graphql -f query=… -f q=… -f after=…`
const fields = {};
for (let i = 0; i < args.length; i++) {
  if (args[i] === "-f" || args[i] === "-F") {
    const [k, ...v] = (args[++i] ?? "").split("=");
    fields[k] = v.join("=");
  }
}
const query = fields.query ?? "";
const kind = !query
  ? ""
  : query.includes("headCommit:")
    ? "open_prs"
    : query.includes("PullRequestReviewThread")
      ? "thread_comments_page"
      : query.includes("on Commit")
        ? "checks_page"
        : query.includes("node(id:") && query.includes("reviewThreads")
          ? "threads_page"
          : query.includes("mergedAt")
            ? "merged_prs"
            : "other";
if (/\bmutation\b/.test(query)) fail(1, "fake gh: mutations are never allowed");

if (process.env.FAKE_GH_LOG) {
  const shown = args.map((a) => (a.startsWith("query=") ? `query=<${kind}>` : a)).join(" ");
  const vars = Object.entries(fields).filter(([k]) => k !== "query").map(([k, v]) => `${k}=${v}`).join(" ");
  appendFileSync(process.env.FAKE_GH_LOG, `${mode} | ${shown} | ${kind} ${vars} | token:${process.env.GH_TOKEN ? "set" : "none"}\n`);
}

const [a0, a1] = args;

if (a0 === "auth" && a1 === "status") {
  if (mode === "logged-out") fail(1, "You are not logged into any GitHub hosts. To log in, run: gh auth login");
  answer(`github.com\n  ✓ Logged in to github.com account ${me} (keyring)\n  - Active account: true\n  - Git operations protocol: ssh\n  - Token: gho_************************************\n`);
}

if (a0 === "auth" && a1 === "token") {
  const user = args[args.indexOf("--user") + 1];
  if (mode === "logged-out" || user !== me) fail(1, `no oauth token found for github.com account ${user}`);
  answer(`${token}\n`);
}

const signedIn = () => {
  if (process.env.GH_TOKEN !== token) fail(4, read("gh-logged-out.stderr"));
};
const failures = {
  "bad-credentials": () => fail(1, read("gh-bad-credentials.stderr")),
  "rate-limit": () => fail(1, "GraphQL: API rate limit exceeded for user ID 1234567. If you reach out to GitHub Support for help, please include the request ID 1:2:3."),
  network: () => fail(1, "error connecting to api.github.com\ncheck your internet connection or https://githubstatus.com"),
};

// FAKE_GH_FAIL_FILE lists `PR=sha` pairs: that PR's latest commit is now `sha`, and a check failed on it (a new push
// whose pipeline failed). Read on every call, like the mode.
const failFile = process.env.FAKE_GH_FAIL_FILE;
const failing = Object.fromEntries(
  (failFile && existsSync(failFile) ? readFileSync(failFile, "utf8") : "")
    .split(/\s+/)
    .filter((x) => x.includes("="))
    .map((x) => x.split("=")),
);
// FAKE_GH_NO_CHECKS=2265,…: those PRs have no checks at all (GitHub gives no rollup; `gh pr checks` exits 1 saying so)
const noChecks = (process.env.FAKE_GH_NO_CHECKS ?? "").split(",").filter(Boolean).map(Number);
const withFailures = (nodes) => {
  for (const n of nodes) {
    if (noChecks.includes(n.number) && n.headCommit?.nodes[0]) n.headCommit.nodes[0].commit.statusCheckRollup = null;
  }
  for (const n of nodes) {
    const sha = failing[String(n.number)];
    const commit = n.headCommit?.nodes[0]?.commit;
    if (!sha || !commit) continue;
    commit.oid = sha;
    commit.statusCheckRollup ??= { state: "FAILURE", contexts: { nodes: [], pageInfo: { hasNextPage: false, endCursor: null } } };
    commit.statusCheckRollup.state = "FAILURE";
    commit.statusCheckRollup.contexts.nodes.push({
      __typename: "CheckRun",
      name: "tests (e2e)",
      conclusion: "FAILURE",
      status: "COMPLETED",
      detailsUrl: `https://ci.example.com/run/${sha.slice(0, 7)}`,
    });
  }
  return nodes;
};

// The commands the live check (Check connections, enkidu doctor) asks directly, answered from the same recorded PRs.
const openNodes = () => withFailures(fixture("gh-open-prs.json").data.search.nodes);
const opt = (name) => {
  const i = args.indexOf(name);
  return i >= 0 ? args[i + 1] : undefined;
};
const bucket = (n) => {
  if (n.__typename === "StatusContext") return n.state === "SUCCESS" ? "pass" : n.state === "PENDING" || n.state === "EXPECTED" ? "pending" : "fail";
  if (n.status !== "COMPLETED") return "pending";
  return { SUCCESS: "pass", CANCELLED: "cancel", SKIPPED: "skipping", NEUTRAL: "skipping" }[n.conclusion] ?? "fail";
};
if (a0 === "pr" && ["list", "checks", "view"].includes(a1)) {
  signedIn();
  failures[mode]?.();
  if (a1 === "list") {
    answer(
      openNodes().map((n) => ({
        number: n.number, title: n.title, isDraft: n.isDraft, headRefName: n.headRefName, baseRefName: n.baseRefName,
        reviewDecision: n.reviewDecision ?? "", mergeable: n.mergeable, state: n.state,
      })),
    );
  }
  const node = openNodes().find((n) => n.number === Number(args[2]));
  if (!node) fail(1, `no pull requests found for branch "${args[2]}"`);
  if (a1 === "checks") {
    const contexts = node.headCommit.nodes[0]?.commit.statusCheckRollup?.contexts.nodes ?? [];
    // what the real gh does for a PR without any checks, even with --json
    if (!contexts.length) fail(1, `no checks reported on the '${node.headRefName}' branch`);
    answer(contexts.map((n) => ({ name: n.name ?? n.context, state: n.conclusion ?? n.state ?? n.status, bucket: bucket(n) })));
  }
  // `pr view --json …`, shaped as the real gh answers it (recorded): each reviewer's latest review, the people (or
  // teams) asked to review, the labels, the size and the author
  const latest = new Map();
  for (const r of node.reviews.nodes) {
    const who = r.author?.login;
    // like GitHub, the PR's author's own replies aren't among its latest reviews
    if (!who || r.state === "PENDING" || who === node.author?.login) continue;
    if (!latest.has(who) || (latest.get(who).submittedAt ?? "") <= (r.submittedAt ?? "")) latest.set(who, r);
  }
  answer({
    comments: node.comments.nodes.map((c) => ({ author: { login: c.author?.login ?? "" }, body: c.body, createdAt: c.createdAt })),
    additions: node.additions,
    deletions: node.deletions,
    changedFiles: node.changedFiles,
    author: { id: "", is_bot: false, login: node.author?.login ?? "", name: "" },
    labels: node.labels.nodes.map((l) => ({ id: "", name: l.name, description: "", color: "" })),
    reviewRequests: node.reviewRequests.nodes
      .map((r) => r.requestedReviewer)
      .filter(Boolean)
      .map((x) => (x.__typename === "Team" ? { __typename: "Team", name: x.name, slug: x.name } : { __typename: x.__typename, login: x.login })),
    latestReviews: [...latest.values()].map((r) => ({ author: { login: r.author.login }, state: r.state, submittedAt: r.submittedAt })),
  });
}

if (a0 === "api" && a1?.startsWith("repos/")) {
  signedIn();
  failures[mode]?.();
  if (mode === "no-access") fail(1, "gh: Not Found (HTTP 404)");
  answer(`${a1.slice("repos/".length)}\n`);
}

if (a0 === "api" && a1 === "graphql") {
  signedIn();
  failures[mode]?.();
  if (mode === "garbage") answer("<!DOCTYPE html><html><body>Unicorn!</body></html>");
  if (mode === "graphql-error")
    fail(1, "GraphQL: Something went wrong while executing your query. This may be the result of a timeout, or it could be a GitHub bug. (search)");
  if (mode === "slow") await new Promise((r) => setTimeout(r, 120_000));
  const pages = mode === "pages";
  const suffix = (v, s) => JSON.parse(JSON.stringify(v), (k, x) => (k === "id" && typeof x === "string" ? `${x}${s}` : x));

  // the live check's own small query: a PR's review threads, resolved or not
  if (query.includes("repository(owner:")) {
    const node = openNodes().find((n) => n.number === Number(fields.p));
    const threads = node?.reviewThreads ?? { totalCount: 0, nodes: [] };
    answer({ data: { repository: { pullRequest: { reviewThreads: { totalCount: threads.totalCount, nodes: threads.nodes.map((t) => ({ isResolved: t.isResolved })) } } } } });
  }
  if (kind === "open_prs") {
    const v = fixture("gh-open-prs.json");
    const search = v.data.search;
    withFailures(search.nodes);
    if (mode === "empty") {
      search.nodes = [];
      search.issueCount = 0;
    }
    if (mode === "missing-field") delete search.nodes[2].reviewDecision;
    if (pages) {
      const red = search.nodes.find((n) => n.number === 2177);
      red.reviewThreads.pageInfo = { hasNextPage: true, endCursor: "threads-2" };
      red.reviewThreads.nodes[0].comments.pageInfo = { hasNextPage: true, endCursor: "comments-2" };
      red.headCommit.nodes[0].commit.statusCheckRollup.contexts.pageInfo = { hasNextPage: true, endCursor: "checks-2" };
      const first = !fields.after;
      search.nodes = first ? search.nodes.slice(0, 3) : search.nodes.slice(3);
      search.pageInfo = first ? { hasNextPage: true, endCursor: "page-2" } : { hasNextPage: false, endCursor: "end" };
    }
    answer(v);
  }
  if (kind === "merged_prs") {
    const v = fixture("gh-merged-prs.json");
    if (mode === "empty") v.data.search.nodes = [];
    answer(v);
  }
  if (kind === "threads_page") {
    const v = suffix(fixture("gh-threads-page.json"), "-p2");
    for (const t of v.data.node.reviewThreads.nodes) t.comments.pageInfo = { hasNextPage: false, endCursor: "x" };
    v.data.node.reviewThreads.pageInfo = { hasNextPage: false, endCursor: "end" };
    answer(v);
  }
  if (kind === "thread_comments_page") {
    const v = suffix(fixture("gh-thread-comments-page.json"), "-p2");
    v.data.node.comments.pageInfo = { hasNextPage: false, endCursor: "end" };
    answer(v);
  }
  if (kind === "checks_page") {
    const v = fixture("gh-checks-page.json");
    const contexts = v.data.node.statusCheckRollup.contexts;
    for (const n of contexts.nodes) if (n.name) n.name = `${n.name} (second page)`;
    contexts.pageInfo = { hasNextPage: false, endCursor: "end" };
    answer(v);
  }
}

fail(1, `fake gh: no answer for: ${args.map((a) => (a.startsWith("query=") ? "query=…" : a)).join(" ")}`);
