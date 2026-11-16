// The sample Work index: invented tickets and PRs in a small shop's code, shaped like what the core sends
// (`enkidu_core::work::model::Snapshot`). It goes with the sample workspaces (w1 to w5, sessions s-w1…) and adds
// sessions started in another terminal, merged PRs in QA, and a bit of everything that can wait on you.
//
// The shapes are the core's own (its `work::model`); they are not spelled out a second time here.
type AdfNode = any;
type PrDetails = any;
type WorkEvent = any;
type WorkFlow = any;
type WorkFlowRun = any;
type WorkFlowTask = any;
type WorkItem = any;
type WorkLink = any;
type WorkPr = any;
type WorkReport = any;
type WorkSession = any;
type WorkSnapshot = any;
type WorkTicket = any;

const REPO = "demo-org/shop";
const MIN = 60_000;

export function sampleWork(now: number, home: string, scenario: string, mode = ""): WorkSnapshot {
  if (scenario !== "main" || mode.includes("first-run")) {
    return { health: [], prs: [], tickets: [], sessions: [], links: [], items: [], events: [], checking: false };
  }
  const iso = (agoMin: number) => new Date(now - agoMin * MIN).toISOString();
  const pr = (number: number, head: string, base: string, title: string, over: Partial<WorkPr> = {}): WorkPr => ({
    projectId: "p1",
    repo: REPO,
    number,
    title,
    url: `https://github.com/${REPO}/pull/${number}`,
    state: "OPEN",
    draft: false,
    head,
    base,
    reviewDecision: "REVIEW_REQUIRED",
    checks: "SUCCESS",
    failing: [],
    running: 0,
    mergeable: "MERGEABLE",
    headSha: `${number}abc0def1`,
    headAt: iso(180),
    unresolved: 0,
    updatedAt: iso(60),
    mergedAt: null,
    commits: [],
    ...over,
  });
  // what the read keeps about each open PR (the card in front of the board shows it)
  const details = (over: Partial<PrDetails>): PrDetails => ({
    author: "sam-lee",
    createdAt: iso(3 * 24 * 60),
    body: "",
    additions: 120,
    deletions: 30,
    changedFiles: 4,
    labels: [],
    checks: [
      { name: "lint", bucket: "pass", url: "https://ci.example.com/run/1" },
      { name: "build", bucket: "pass", url: "https://ci.example.com/run/2" },
      { name: "tests (3.12)", bucket: "pass", url: "https://ci.example.com/run/3" },
    ],
    reviews: [],
    requested: [],
    threads: [],
    comments: [],
    commitCount: 2,
    recentCommits: [
      { oid: "a1b2c3d", headline: "Address the review" },
      { oid: "e4f5a6b", headline: "First version" },
    ],
    ...over,
  });
  const thread = (path: string, line: number, who: string, text: string, over: Partial<PrDetails["threads"][number]> = {}): PrDetails["threads"][number] => ({
    path,
    line,
    resolved: false,
    outdated: false,
    who,
    bot: false,
    text,
    replies: 0,
    lastWho: null,
    lastAt: null,
    url: `https://github.com/${REPO}/pull/2414#discussion_r${line}`,
    ...over,
  });
  const prs: WorkPr[] = [
    pr(2414, "feature/SH-21309-export", "development", "SH-21309 Export orders to the accounting CSV", {
      checks: "FAILURE",
      failing: [{ name: "tests (3.12)", state: "FAILURE", url: "https://ci.example.com/run/88123" }],
      unresolved: 2,
      updatedAt: iso(12),
      headAt: iso(95),
      details: details({
        createdAt: iso(2 * 24 * 60),
        body: EXPORT_PR,
        additions: 412,
        deletions: 38,
        changedFiles: 9,
        labels: ["accounting"],
        checks: [
          { name: "tests (3.12)", bucket: "fail", url: "https://ci.example.com/run/88123" },
          { name: "tests (3.11)", bucket: "pass", url: "https://ci.example.com/run/88122" },
          { name: "lint", bucket: "pass", url: "https://ci.example.com/run/88121" },
          { name: "build", bucket: "pass", url: "https://ci.example.com/run/88120" },
          { name: "SonarCloud", bucket: "pass", url: "https://ci.example.com/sonar" },
          { name: "deploy preview", bucket: "skipping", url: null },
        ],
        reviews: [
          { who: "mira-k", state: "COMMENTED", at: iso(40), bot: false },
          { who: "coderabbitai", state: "COMMENTED", at: iso(2 * 24 * 60 - 20), bot: true },
        ],
        requested: ["tom-b"],
        threads: [
          thread("orders/export.py", 48, "mira-k", "The CSV writer should use the outlet's decimal separator, accounting imports these with a comma in France."),
          thread("orders/export.py", 112, "mira-k", "Rows with a zero total shouldn't be exported at all.", { replies: 1, lastWho: "sam-lee", lastAt: iso(100) }),
          thread("orders/views.py", 20, "coderabbitai", "Consider streaming the response instead of building the whole file in memory.", { resolved: true, bot: true, replies: 2, lastWho: "sam-lee", lastAt: iso(26 * 60) }),
        ],
        comments: [
          { who: "coderabbitai", bot: true, text: "Walkthrough: adds a CSV export of the filtered orders, with one row per order line.", at: iso(2 * 24 * 60 - 25), url: `https://github.com/${REPO}/pull/2414#issuecomment-1` },
          { who: "jonas-qa", bot: false, text: "Does the export include orders that were cancelled after delivery?", at: iso(14), url: `https://github.com/${REPO}/pull/2414#issuecomment-2` },
        ],
        commitCount: 5,
        recentCommits: [
          { oid: "4973abc", headline: "Skip rows with a zero total" },
          { oid: "91c2d0e", headline: "Use the outlet's locale for numbers" },
          { oid: "77e1f3a", headline: "Stream the file" },
          { oid: "3b8a9c1", headline: "Add the export action to the order list" },
          { oid: "1f0e2d4", headline: "Order lines as CSV rows" },
        ],
      }),
    }),
    pr(2395, "fix/stock-double-count", "development", "SH-21533 Stock counts twice after a return", {
      reviewDecision: "CHANGES_REQUESTED",
      unresolved: 1,
      updatedAt: iso(38),
      headAt: iso(240),
      details: details({
        body: "Returns to a supplier booked the stock twice: once on the return and again on the credit note.\n\n## Fix\n- The credit note no longer moves stock\n- `StockLevel` is refreshed after `save()`",
        additions: 64,
        deletions: 12,
        changedFiles: 3,
        reviews: [
          { who: "mira-k", state: "CHANGES_REQUESTED", at: iso(38), bot: false },
          { who: "coderabbitai", state: "COMMENTED", at: iso(230), bot: true },
        ],
        threads: [
          thread("stock/levels.py", 21, "mira-k", "After save() this is still an F expression; return the refreshed level."),
          thread("stock/levels.py", 30, "coderabbitai", "Consider wrapping the two updates in a transaction.", { bot: true }),
        ],
        commitCount: 3,
      }),
    }),
    pr(2413, "hotfix/SH-21533-stock-double-count", "master", "SH-21533 Hotfix: stock counts twice after a return", {
      reviewDecision: "APPROVED",
      checks: "PENDING",
      running: 3,
      updatedAt: iso(6),
      headAt: iso(7),
      details: details({
        body: "The same fix as #2395, for the release on master.",
        additions: 41,
        deletions: 9,
        changedFiles: 2,
        checks: [
          { name: "tests (3.12)", bucket: "pending", url: "https://ci.example.com/run/90001" },
          { name: "lint", bucket: "pending", url: "https://ci.example.com/run/90002" },
          { name: "build", bucket: "pending", url: "https://ci.example.com/run/90003" },
        ],
        reviews: [{ who: "mira-k", state: "APPROVED", at: iso(20), bot: false }],
        commitCount: 1,
        recentCommits: [{ oid: "4964abc", headline: "Hotfix: stock counts twice after a return" }],
      }),
    }),
    pr(2375, "feature/SH-21376-bundle-units", "development", "SH-21376 Units on bundles", {
      updatedAt: iso(26 * 60),
      details: details({ body: "Bundles keep the unit each component was entered in.", requested: ["mira-k", "tom-b"] }),
    }),
    pr(2326, "feature/SH-21112-bundles", "development", "SH-21112 Bundles show the cost per article", {
      state: "MERGED",
      reviewDecision: "APPROVED",
      mergedAt: iso(30 * 60),
      updatedAt: iso(30 * 60),
    }),
    pr(2390, "feature/SH-21616-vat-line", "development", "SH-21616 Dispatch note PDF shows the VAT line", {
      state: "MERGED",
      reviewDecision: "APPROVED",
      mergedAt: iso(3 * 24 * 60),
      updatedAt: iso(3 * 24 * 60),
    }),
    pr(2312, "feature/SH-21030-warranty-labels", "development", "SH-21030 Warranty labels on the order PDF", {
      state: "MERGED",
      reviewDecision: "APPROVED",
      mergedAt: iso(2 * 24 * 60),
      updatedAt: iso(2 * 24 * 60),
    }),
    // a PR that names no ticket stands where its state puts it (here: the review column)
    pr(2415, "chore/django-5.2.8", "development", "Bump Django to 5.2.8", {
      updatedAt: iso(3 * 60),
      details: details({ body: "Security release.", additions: 2, deletions: 2, changedFiles: 1, labels: ["dependencies"], requested: ["mira-k"], commitCount: 1, recentCommits: [{ oid: "4977d0c", headline: "Bump Django to 5.2.8" }] }),
    }),
    // a second project's PR: the board offers one project at a time, or all
    {
      ...pr(212, "feature/work-board", "main", "Board view for the Work page", { draft: true, checks: "PENDING", running: 2, updatedAt: iso(20) }),
      projectId: "p2",
      repo: "demo-org/enkidu",
      url: "https://github.com/demo-org/enkidu/pull/212",
    },
  ];
  const ticket = (key: string, summary: string, status: string, category: string, updatedMin: number, over: Partial<WorkTicket> = {}): WorkTicket => ({
    key,
    projectKey: "SH",
    summary,
    url: `https://demo.atlassian.net/browse/${key}`,
    status,
    category,
    assignee: "Sam Lee",
    assigneeId: "acc-me",
    updatedAt: iso(updatedMin),
    comments: 0,
    attachments: 0,
    ...over,
  });
  const tickets: WorkTicket[] = [
    ticket("SH-21309", "Export orders to the accounting CSV", "In Progress", "indeterminate", 12, { issueType: "Story", priority: "High", description: EXPORT_DOC }),
    ticket("SH-21533", "Stock counts twice after a return", "Code Review", "indeterminate", 38, { issueType: "Bug", priority: "Highest", description: STOCK_DOC }),
    ticket("SH-21112", "Bundles show the cost per article", "QA Review", "indeterminate", 45, { comments: 4, attachments: 2, issueType: "Task", description: doc(para("Show the cost of one article next to the bundle's total cost, using the article count on the bundle.")) }),
    ticket("SH-21616", "Dispatch note PDF misses the VAT line", "In Progress", "indeterminate", 130, { comments: 3, issueType: "Bug", description: doc(para("The VAT line is missing on dispatch notes for suppliers outside France.")) }),
    ticket("SH-21376", "Units on bundles", "Code Review", "indeterminate", 26 * 60, { issueType: "Task" }),
    ticket("SH-21475", "Sales report per outlet", "On Hold", "indeterminate", 3 * 24 * 60, { issueType: "Task", priority: "Low", description: doc(para("Waiting for the product team to decide which sales channels count.")) }),
    ticket("SH-21522", "Planned orders keep the old delivery day", "Planned for current sprint", "indeterminate", 5 * 60, { issueType: "Bug" }),
    ticket("SH-21030", "Warranty labels on the order PDF", "Done", "done", 2 * 24 * 60, { issueType: "Story" }),
    ticket("SH-20728", "Supplier price import", "Ready for Dev", "new", 4 * 24 * 60, { issueType: "Story" }),
    ticket("SH-20781", "Price import: keep the supplier's own units", "Ready for Dev", "new", 4 * 24 * 60, { issueType: "Sub-task" }),
    ticket("SH-20947", "Inventory count on a tablet", "Ready for Dev", "new", 9 * 24 * 60, { issueType: "Epic" }),
    ticket("SH-21205", "Search bundles by warranty", "Analysed", "new", 6 * 24 * 60, { issueType: "Story", priority: "Lowest" }),
    ticket("SH-21580", "Outlet groups in the stock report", "New", "new", 26 * 60, { issueType: "Task" }),
  ];
  const session = (id: string, title: string, cwd: string, branches: string[], lastMin: number, over: Partial<WorkSession> = {}): WorkSession => ({
    id,
    configDir: `${home}/.claude-work`,
    cwd,
    title,
    branches,
    workspaceId: null,
    firstSeen: now - (lastMin + 300) * MIN,
    lastSeen: now - lastMin * MIN,
    source: "transcript",
    ...over,
  });
  const main = `${home}/code/shop`;
  const wsDir = `${home}/enkidu/workspaces/shop`;
  const sessions: WorkSession[] = [
    session("s-w1", "stock-fix", `${wsDir}/stock-fix`, ["fix/stock-double-count", "hotfix/SH-21533-stock-double-count"], 2, { workspaceId: "w1", source: "workspace" }),
    session("s-w2", "sh-21309-export", `${wsDir}/sh-21309-export`, ["feature/SH-21309-export"], 1, { workspaceId: "w2", source: "workspace" }),
    session("s-w3", "pr-remarks-2175", `${wsDir}/pr-remarks-2175`, ["feature/SH-21112-bundles"], 40, { workspaceId: "w3", source: "workspace" }),
    session("8b9a2f10-55d1-4e0b-9a51-2c1d7e6f0a11", "units on bundles", main, ["feature/SH-21376-bundle-units"], 26 * 60),
    session("3d5c2d6e-0c4f-4b7e-8d2a-91f0e3b4c5a6", "vat line on dispatch notes", main, ["development", "feature/SH-21616-vat-line"], 3 * 24 * 60),
  ];
  const link = (a: string, b: string, how: string, detail: string, agoMin: number): WorkLink => ({ a, b, how, detail, at: now - agoMin * MIN, removed: false });
  const p = (n: number) => `pr:${REPO}#${n}`;
  const links: WorkLink[] = [
    link("ticket:SH-21309", p(2414), "ticket-key", "the key is in its branch", 200),
    link("ticket:SH-21533", p(2395), "ticket-key", "the key is in its title", 300),
    link("ticket:SH-21533", p(2413), "ticket-key", "the key is in its branch", 20),
    link("ticket:SH-21376", p(2375), "ticket-key", "the key is in its branch", 26 * 60),
    link("ticket:SH-21112", p(2326), "ticket-key", "the key is in its branch", 40 * 60),
    link("ticket:SH-21616", p(2390), "ticket-key", "the key is in its branch", 4 * 24 * 60),
    link("ticket:SH-21030", p(2312), "ticket-key", "the key is in its branch", 3 * 24 * 60),
    link(p(2414), "session:s-w2", "branch", "worked on its branch", 200),
    link(p(2414), "workspace:w2", "branch", "its workspace", 200),
    link(p(2395), "session:s-w1", "hook:pr-create", "created it", 300),
    link(p(2395), "workspace:w1", "branch", "its workspace", 300),
    link(p(2413), "session:s-w1", "hook:pr-create", "created it", 20),
    link(p(2326), "session:s-w3", "branch", "worked on its branch", 40 * 60),
    link(p(2326), "workspace:w3", "branch", "its workspace", 40 * 60),
    link(p(2375), "session:8b9a2f10-55d1-4e0b-9a51-2c1d7e6f0a11", "transcript", "created it", 26 * 60),
    link(p(2390), "session:3d5c2d6e-0c4f-4b7e-8d2a-91f0e3b4c5a6", "transcript", "pushed its branch", 3 * 24 * 60),
  ];
  const item = (id: string, kind: WorkItem["kind"], over: Partial<WorkItem>): WorkItem => ({
    id,
    kind,
    source: "github",
    projectId: "p1",
    pr: null,
    ticket: null,
    title: "",
    detail: "",
    who: null,
    url: null,
    path: null,
    line: null,
    at: iso(30),
    state: "open",
    openedAt: now - 30 * MIN,
    closedAt: null,
    snoozedUntil: null,
    bot: false,
    ...over,
  });
  const r = (n: number) => `${REPO}#${n}`;
  const items: WorkItem[] = [
    item(`checks:${r(2414)}:4973abc0def1`, "pipeline_failed", {
      pr: r(2414),
      title: "Checks failing: tests (3.12)",
      detail: "tests (3.12): failure",
      url: "https://ci.example.com/run/88123",
      at: iso(90),
    }),
    item("thread:t1", "remark", {
      pr: r(2414),
      title: "Remark from mira-k on orders/export.py:48",
      detail: "The CSV writer should use the outlet's decimal separator, accounting imports these with a comma in France.",
      who: "mira-k",
      path: "orders/export.py",
      line: 48,
      at: iso(40),
    }),
    item("thread:t2", "probably_fixed", {
      pr: r(2414),
      title: "Probably fixed, needs a reply: mira-k on orders/export.py:112",
      detail: "Rows with a zero total shouldn't be exported at all.",
      who: "mira-k",
      path: "orders/export.py",
      line: 112,
      at: iso(150),
    }),
    item("question:c9", "question", {
      pr: r(2414),
      title: "Question from jonas-qa",
      detail: "Does the export include orders that were cancelled after delivery?",
      who: "jonas-qa",
      at: iso(14),
    }),
    item("changes:r7", "changes_requested", { pr: r(2395), title: "Changes requested by mira-k", who: "mira-k", at: iso(38) }),
    item("thread:t5", "remark", {
      pr: r(2395),
      title: "Remark from mira-k on stock/levels.py:21",
      detail: "After save() this is still an F expression; return the refreshed level.",
      who: "mira-k",
      path: "stock/levels.py",
      line: 21,
      at: iso(38),
    }),
    item("thread:t6", "remark", {
      pr: r(2395),
      title: "Remark from coderabbitai on stock/levels.py:30",
      detail: "Consider wrapping the two updates in a transaction.",
      who: "coderabbitai",
      path: "stock/levels.py",
      line: 30,
      at: iso(230),
      bot: true,
    }),
    item("jira-comment:SH-21112:9001", "qa_replied", {
      source: "jira",
      ticket: "SH-21112",
      title: "QA replied: Jonas Weber",
      detail: "If a bundle uses a sub-bundle without a quantity, the cost per article shows NaN. Screenshot attached.",
      who: "Jonas Weber",
      at: iso(45),
    }),
    item("jira-attachment:SH-21112:777", "attachment", {
      source: "jira",
      ticket: "SH-21112",
      title: "New attachment: cost-per-article-nan.png",
      detail: "Jonas Weber attached a screenshot",
      who: "Jonas Weber",
      at: iso(44),
    }),
    item("jira-back:SH-21616", "back_to_you", {
      source: "jira",
      ticket: "SH-21616",
      title: "Back to you: QA Review → In Progress",
      detail: "Moved back by Jonas Weber",
      who: "Jonas Weber",
      at: iso(130),
    }),
  ];
  const ev = (subject: string, agoMin: number, kind: string, text: string): WorkEvent => ({ subject, at: now - agoMin * MIN, kind, text });
  const events: WorkEvent[] = [
    ev(r(2413), 7, "pushed", "Pushed 4964abc"),
    ev(r(2414), 14, "item-opened", "Question from jonas-qa"),
    ev(r(2395), 38, "review", "Changes requested"),
    ev("SH-21112", 45, "item-opened", "QA replied: Jonas Weber"),
    ev(r(2414), 90, "checks", "Checks failed: tests (3.12)"),
    ev(r(2414), 95, "pushed", "Pushed 4973abc"),
    ev("SH-21616", 130, "status", "QA Review → In Progress"),
  ];
  const health: WorkSnapshot["health"] = [
    { projectId: "p1", source: "github" as const, state: "ok" as const, lastOk: now - 2 * MIN, lastTry: now - 2 * MIN, error: null, fix: null, nextTry: now + 3 * MIN, account: "sam-lee" },
    { projectId: "p1", source: "jira" as const, state: "ok" as const, lastOk: now - 2 * MIN, lastTry: now - 2 * MIN, error: null, fix: null, nextTry: now + 3 * MIN, account: "Sam Lee" },
  ];
  if (mode.includes("broken-jira") || mode.includes("broken-both")) {
    health[1] = { ...health[1], state: "error", lastOk: now - 42 * MIN, error: "Jira: twg isn't signed in", fix: "Run `twg login --force`.", nextTry: now + 2 * MIN };
  }
  if (mode.includes("broken-both")) {
    health[0] = { ...health[0], state: "error", lastOk: now - 9 * MIN, error: "No connection to GitHub", fix: "Check the internet connection; Enkidu tries again.", nextTry: now + MIN };
  }
  const ticketProjects = Object.fromEntries(tickets.map((t) => [t.key, "p1"]));
  // ?work=hidden: a few cards you hid (one with something waiting on you: the Hidden strip shows its dot)
  const hidden = mode.includes("hidden") ? ["ticket:SH-21475", "ticket:SH-21376", "ticket:SH-21616", "pr:demo-org/shop#2415"] : [];
  // Jira's boards (the choice in the header); ?work=night: the morning after a night's workflow, its reports on the
  // tasks, and the next one due tonight
  const boards = [
    { id: "5", name: "Shop Dev", kind: "scrum", projectKey: "SH" },
    { id: "9", name: "Release view", kind: "kanban", projectKey: "SH" },
    { id: "20", name: "Integrations", kind: "kanban", projectKey: "SH" },
  ];
  // ?work=night: a workflow that runs now (its first task reported, the second running and saying how it goes, two
  // waiting their turn), last night's run, and one due tonight; their reports and updates, screenshots included
  const night = mode.includes("night");
  const HOUR = 60 * MIN;
  const DAY = 24 * HOUR;
  const reports: WorkReport[] = night
    ? [
        {
          id: 14,
          subject: "SH-21309",
          sessionId: "s-w2",
          workspaceId: "w2",
          state: "update",
          text: "Fixed the separator: the CSV writer now takes the outlet's locale. Checking the export in the dev environment now.",
          images: ["1-export-half.png"],
          at: now - 6 * MIN,
        },
        {
          id: 13,
          subject: "SH-21309",
          sessionId: "s-w2",
          workspaceId: "w2",
          state: "update",
          text: "Found it: QA's file came from a French outlet, and the CSV writer ignores the outlet's locale. The date column is fine.",
          images: [],
          at: now - 20 * MIN,
        },
        {
          id: 11,
          subject: "SH-21533",
          sessionId: "s-w1",
          workspaceId: "w1",
          state: "done",
          text: "Fixed the double count QA found: a stock movement saved twice by the retry is now skipped by its idempotency key. The test that reproduced it passes, and so do the 214 stock tests.",
          images: ["1-stock-before-after.png"],
          at: now - 33 * MIN,
        },
        {
          id: 12,
          subject: "SH-21309",
          sessionId: "s-w2",
          workspaceId: "w2",
          state: "review",
          text: "Added **Export to CSV** to the order list: one row per order line, with the outlet's decimal separator.\n\n- Checked an export of the 31 orders of October in the dev environment: the totals match the order list.\n- Cancelled orders are left out (two in October).\n- Not done: 10,000 lines took 7 s, over the 5 s the ticket asks; the query loads every line's product. Worth your look before I change it.",
          images: ["1-order-list.png", "2-export-in-excel.png", "3-totals.png"],
          at: now - 26 * HOUR,
        },
        // two nights ago: one done with its screenshot, one stuck
        {
          id: 9,
          subject: "SH-21580",
          sessionId: "s-w1",
          workspaceId: "w1",
          state: "done",
          text: "Outlet groups now show in the stock count: each outlet's count rolls up into its group, and the group total matches the sum of its outlets (checked on the Ghent group, 3 outlets). Added 6 tests.",
          images: ["1-outlet-groups.png"],
          at: now - 2 * DAY - 2 * HOUR - 10 * MIN,
        },
        {
          id: 8,
          subject: "SH-21522",
          sessionId: "s-w2",
          workspaceId: "w2",
          state: "blocked",
          text: "Stuck: a planned order takes its delivery day from the supplier's calendar, which comes from the Integrations service. It doesn't run in the small stack, so I can't reproduce the old day locally. Could you start it, or tell me which supplier QA used?",
          images: [],
          at: now - 2 * DAY - 70 * MIN,
        },
      ]
    : [];
  const task = (
    key: string,
    title: string,
    workspaceId: string,
    state: WorkFlowTask["state"],
    kind: WorkFlowTask["kind"],
    over: Partial<WorkFlowTask> = {},
  ): WorkFlowTask => ({
    subject: `ticket:${key}`,
    title,
    target: { kind: "workspace", workspaceId },
    message: `${key}: ${title}`,
    edited: false,
    kind,
    instruction: "",
    state,
    workspaceId,
    note: null,
    ...over,
  });
  const flows: WorkFlow[] = night
    ? [
        {
          id: "f3",
          projectId: "p1",
          name: "Night run",
          instruction: "Use the small Docker stack; leave the migrations alone.",
          startAt: now - 70 * MIN,
          minutes: 60,
          live: true,
          state: "running",
          current: 1,
          tasks: [
            task("SH-21533", "Stock counts twice after a return", "w1", "done", "came_back", { startedAt: now - 70 * MIN, endedAt: now - 32 * MIN }),
            task("SH-21309", "Export orders to the accounting CSV", "w2", "running", "came_back", { startedAt: now - 32 * MIN, minutes: 45 }),
            task("SH-21376", "Units on bundles", "w1", "waiting", "continue", { workspaceId: null }),
            task("SH-21475", "Sales report per outlet", "w2", "waiting", "investigate", { workspaceId: null, minutes: 30 }),
          ],
          updatedAt: now - 32 * MIN,
        },
        {
          id: "f2",
          projectId: "p1",
          name: "Tonight",
          instruction: "",
          startAt: (() => {
            const d = new Date(now);
            d.setHours(2, 0, 0, 0);
            if (d.getTime() <= now) d.setDate(d.getDate() + 1);
            return d.getTime();
          })(),
          minutes: 60,
          live: true,
          state: "scheduled",
          tasks: [task("SH-21522", "Planned orders keep the old delivery day", "w1", "waiting", "start", { workspaceId: null })],
          updatedAt: now,
        },
        {
          id: "f1",
          projectId: "p1",
          name: "Last night",
          instruction: "",
          startAt: now - 27 * HOUR,
          minutes: 60,
          live: true,
          state: "done",
          tasks: [
            task("SH-21309", "Export orders to the accounting CSV", "w2", "review", "start", { startedAt: now - 27 * HOUR, endedAt: now - 26 * HOUR }),
            task("SH-21475", "Sales report per outlet", "w2", "timed_out", "investigate", {
              startedAt: now - 26 * HOUR,
              endedAt: now - 25 * HOUR,
              note: "Its 60 minutes ran out before Claude reported; it may still be working",
            }),
          ],
          updatedAt: now - 25 * HOUR,
        },
      ]
    : [];
  // the runs kept: last night's, the Night run two nights ago (one done, one stuck, one without a report), and a weekend
  // run that didn't start (the Mac was off)
  const twoNights = now - 2 * DAY - 3 * HOUR;
  const flowRuns: WorkFlowRun[] = night
    ? [
        { id: 3, flowId: "f1", startedAt: now - 27 * HOUR, endedAt: now - 25 * HOUR, state: "done", flow: flows.find((f) => f.id === "f1")! },
        {
          id: 2,
          flowId: "f3",
          startedAt: twoNights,
          endedAt: twoNights + 2 * HOUR + 35 * MIN,
          state: "done",
          flow: {
            ...flows.find((f) => f.id === "f3")!,
            state: "done",
            current: null,
            tasks: [
              task("SH-21580", "Outlet groups in the stock count", "w1", "done", "start", { startedAt: twoNights, endedAt: twoNights + 52 * MIN }),
              task("SH-21522", "Planned orders keep the old delivery day", "w2", "blocked", "investigate", { startedAt: twoNights + 52 * MIN, endedAt: twoNights + 112 * MIN }),
              task("SH-21376", "Units on bundles", "w1", "no_report", "continue", {
                startedAt: twoNights + 112 * MIN,
                endedAt: twoNights + 155 * MIN,
                note: "Claude stopped without a report, and didn't give one when asked",
              }),
            ],
          },
        },
        {
          id: 1,
          flowId: "f0",
          startedAt: null,
          endedAt: now - 3 * DAY,
          state: "missed",
          flow: {
            id: "f0",
            projectId: "p1",
            name: "Weekend run",
            instruction: "",
            startAt: now - 3 * DAY - 30 * MIN,
            minutes: 60,
            live: true,
            state: "missed",
            tasks: [task("SH-21475", "Sales report per outlet", "w2", "waiting", "investigate", { workspaceId: null })],
          },
        },
      ]
    : [];
  return { health, prs, tickets, ticketProjects, hidden, boards, board: null, sessions, links, items, events, reports, flows, flowRuns, checking: mode.includes("reading") };
}

// Descriptions in Jira's document format, as twg sends them.
const text = (t: string, ...marks: string[]): AdfNode => ({ type: "text", text: t, ...(marks.length ? { marks: marks.map((type) => ({ type })) } : {}) });
const para = (...parts: (string | AdfNode)[]): AdfNode => ({ type: "paragraph", content: parts.map((x) => (typeof x === "string" ? text(x) : x)) });
const doc = (...content: AdfNode[]): AdfNode => ({ type: "doc", content });
const h = (t: string): AdfNode => ({ type: "heading", attrs: { level: 3 }, content: [text(t)] });
const ul = (...items: AdfNode[]): AdfNode => ({ type: "bulletList", content: items.map((i) => ({ type: "listItem", content: [i] })) });

const EXPORT_DOC = doc(
  para("Accounting imports the orders of a month as a CSV. Today they copy them by hand from the order list, which takes a day for a big group."),
  h("What to build"),
  ul(
    para("An ", text("Export to CSV", "strong"), " action on the order list, for the orders that are filtered."),
    para("One row per order line: date, outlet, supplier, product, quantity, unit price, VAT rate, total."),
    para("Numbers use the outlet's decimal separator (", text("1.234,50", "code"), " in France)."),
  ),
  h("Acceptance criteria"),
  { type: "taskList", content: [
    { type: "taskItem", attrs: { state: "DONE" }, content: [text("The file opens in Excel with the right columns")] },
    { type: "taskItem", attrs: { state: "TODO" }, content: [text("Cancelled orders are left out")] },
    { type: "taskItem", attrs: { state: "TODO" }, content: [text("An export of 10,000 lines takes under 5 seconds")] },
  ] },
  para("The columns accounting asked for are in ", { type: "text", text: "the export spec", marks: [{ type: "link", attrs: { href: "https://example.com/export-spec" } }] }, "."),
);

const EXPORT_PR = `Accounting imports the orders of a month as a CSV; this adds the export to the order list.

## What changes
- An **Export to CSV** action on the order list, for the filtered orders
- One row per order line, numbers in the outlet's format (\`1.234,50\` in France)
- Cancelled orders are left out

## How to test
1. Filter the order list to last month
2. Export, and open the file in Excel
- [x] Columns as accounting asked
- [ ] 10,000 lines in under 5 seconds

<!-- the template's reminders: never shown -->
Closes SH-21309`;

const STOCK_DOC = doc(
  para("After a return to the supplier, the stock level of the product drops twice: once when the return is made and again when the credit note comes in."),
  h("Steps"),
  { type: "orderedList", content: [
    { type: "listItem", content: [para("Make a return of 2 kg tomatoes to a supplier.")] },
    { type: "listItem", content: [para("Book the supplier's credit note.")] },
    { type: "listItem", content: [para("The stock shows 4 kg less instead of 2 kg.")] },
  ] },
  { type: "codeBlock", attrs: { language: "python" }, content: [text("StockLevel.objects.filter(item=item).update(quantity=F(\"quantity\") - amount)")] },
);
