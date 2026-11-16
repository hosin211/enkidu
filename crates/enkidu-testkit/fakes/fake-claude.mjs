#!/usr/bin/env node
// A stand-in for the claude CLI in tests. It reads the --settings file Enkidu passes,
// fires the same hooks real Claude Code fires, and records how it was started.
//
// Type a line to "prompt" it. Special prompts:
//   ask     -> asks for permission (needs you), then "y" lets it continue
//   spin N  -> works for N seconds with a spinner, then stops printing without any hook: what an interrupted Claude
//              Code does (Esc runs no Stop hook)
//   exit    -> ends the session
// Every other prompt edits fake-output.txt in the workspace and finishes. One with FAKE_LIMIT in it (a workflow task's
// words, in tests) edits, then hits the usage limit: Claude Code ends the turn on that API error with its StopFailure
// hooks, never Stop. Background hooks ("async") run in order here, like the rest.
import { execSync } from "node:child_process";
import { appendFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import readline from "node:readline";

const argv = process.argv.slice(2);
// What Setup asks before anything else: the version, and whether someone is signed in (as `claude auth status`
// answers, in JSON; here nobody is signed in when the account's folder holds a file `signed-out`). Neither starts a
// session, so neither is logged as one.
if (argv[0] === "--version") {
  process.stdout.write("2.1.325 (Claude Code)\n");
  process.exit(0);
}
if (argv[0] === "auth" && argv[1] === "status") {
  const dir = process.env.CLAUDE_CONFIG_DIR || join(tmpdir(), "fake-claude-config");
  const signedIn = !existsSync(join(dir, "signed-out"));
  process.stdout.write(JSON.stringify(signedIn ? { loggedIn: true, authMethod: "claude.ai", email: "someone@example.com" } : { loggedIn: false, authMethod: "none" }) + "\n");
  process.exit(signedIn ? 0 : 1);
}
const flag = (name) => {
  const i = argv.indexOf(name);
  return i >= 0 ? argv[i + 1] : undefined;
};
const settingsArg = flag("--settings");
// `let`: `/resume` switches to another conversation, as in Claude Code
let sessionId = flag("--session-id") ?? flag("--resume") ?? "no-session";
const resumed = argv.includes("--resume");
let cwd = process.cwd();

let settings = {};
if (settingsArg) {
  settings = JSON.parse(settingsArg.trim().startsWith("{") ? settingsArg : readFileSync(settingsArg, "utf8"));
}

// Where real Claude keeps transcripts, but never the real ~/.claude in tests.
const configDir = process.env.CLAUDE_CONFIG_DIR || join(tmpdir(), "fake-claude-config");
let projectDir = join(configDir, "projects", cwd.replace(/[^A-Za-z0-9]/g, "-"));
let transcript = join(projectDir, `${sessionId}.jsonl`);

if (process.env.FAKE_CLAUDE_LOG) {
  const env = Object.fromEntries(
    Object.entries(process.env)
      .filter(([k]) => k.startsWith("FAKE_") || k.startsWith("ENKIDU_") || k === "CLAUDE_CONFIG_DIR")
      // the app's local token is a secret too: logged as there, not as its value
      .map(([k, v]) => [k, k === "ENKIDU_TOKEN" ? "(set)" : v]),
  );
  appendFileSync(process.env.FAKE_CLAUDE_LOG, JSON.stringify({ argv, cwd, env, sessionId, resumed }) + "\n");
}

// `-p`: print mode, as the morning summary runs it. The prompt comes on stdin, the answer goes to stdout, and it ends.
// Like the real one it runs only what --tools and --allowedTools allow (here: enkidu), and it saves nothing, anywhere.
if (argv.includes("-p") || argv.includes("--print")) {
  const prompt = readFileSync(0, "utf8");
  if (process.env.FAKE_CLAUDE_LOG) appendFileSync(`${process.env.FAKE_CLAUDE_LOG}.prompts`, JSON.stringify({ sessionId: "print", prompt }) + "\n");
  const tools = (flag("--tools") ?? "").split(",");
  const allowed = (flag("--allowedTools") ?? "").split(",");
  if (!tools.includes("Bash") || !allowed.includes("Bash(enkidu:*)")) {
    process.stdout.write("I can't run enkidu here, so I wrote nothing.\n");
    process.exit(0);
  }
  const quote = (s) => `'${s.replace(/'/g, "'\\''")}'`;
  const run = (cmd) => execSync(cmd, { encoding: "utf8", shell: "/bin/sh", env: { ...process.env, CLAUDECODE: "1" }, timeout: 60_000 });
  let tickets;
  try {
    tickets = JSON.parse(run("enkidu list tickets --json"));
  } catch (e) {
    process.stderr.write(`enkidu list tickets failed: ${e.stderr ?? e.message}\n`);
    process.exit(1);
  }
  for (const t of tickets) run(`enkidu note --ticket ${t.key} ${quote(`${t.status}: summarized by the fake Claude`)}`);
  process.stdout.write(`Summarized ${tickets.length} tickets.\n`);
  process.exit(0);
}

function runHooks(event, extra = {}, toolName) {
  const groups = settings.hooks?.[event] ?? [];
  const input = JSON.stringify({ session_id: sessionId, transcript_path: transcript, cwd, hook_event_name: event, ...extra });
  let out = "";
  for (const group of groups) {
    if (group.matcher && toolName !== undefined && !new RegExp(`^(?:${group.matcher})$`).test(toolName)) continue;
    if (group.matcher && toolName === undefined && extra.notification_type !== undefined) {
      if (!new RegExp(`^(?:${group.matcher})$`).test(extra.notification_type)) continue;
    }
    for (const hook of group.hooks ?? []) {
      try {
        out += execSync(hook.command, { input, shell: "/bin/sh", encoding: "utf8", timeout: (hook.timeout ?? 60) * 1000 });
      } catch {
        // a failing hook doesn't stop Claude
      }
    }
  }
  return out.trim();
}

function saveTurn(prompt) {
  mkdirSync(projectDir, { recursive: true });
  appendFileSync(transcript, JSON.stringify({ type: "user", prompt, at: Date.now() }) + "\n");
  // every prompt, whole, for tests that check what reached Claude
  if (process.env.FAKE_CLAUDE_LOG) appendFileSync(`${process.env.FAKE_CLAUDE_LOG}.prompts`, JSON.stringify({ sessionId, prompt }) + "\n");
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// a 1×1 PNG, the fake's screenshot
const PNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/** Claude's turn ends: its Stop hooks run. When one answers {"decision": "block", "reason": …}, Claude goes on with
 * the reason, as Claude Code does; asked for a report (a workflow's task), the fake makes one with a screenshot
 * (FAKE_REPORT: done, review or blocked; "never" leaves it out), then stops again, its Stop hooks told it already went
 * on once (`stop_hook_active`). */
function stopped(prompt = "") {
  // asked for its report in the prompt itself (the workflow's reminder, typed in): it reports, then stops
  if (prompt.includes("you stopped without saying how")) {
    report(prompt);
    runHooks("Stop", { stop_hook_active: false });
    return;
  }
  const out = runHooks("Stop", { stop_hook_active: false });
  let answer = null;
  try {
    answer = out ? JSON.parse(out) : null;
  } catch {
    // an answer that isn't JSON is no decision
  }
  if (answer?.decision !== "block" || typeof answer.reason !== "string") return;
  process.stdout.write(`[stop hook] ${answer.reason.split("\n")[0]}\r\n`);
  report(answer.reason);
  runHooks("Stop", { stop_hook_active: true });
}

/** `enkidu report` as the words asking for it say (`--ticket KEY`, `--pr N`, or `--task N` for a task of your own), with a screenshot. */
function report(asking) {
  const named = /enkidu report done\|review\|blocked (--ticket \S+|--pr \d+|--task \d+)/.exec(asking)?.[1];
  const state = process.env.FAKE_REPORT ?? "done";
  if (named && state !== "never") {
    const shot = join(cwd, "fake-shot.png");
    writeFileSync(shot, Buffer.from(PNG, "base64"));
    let said = "";
    try {
      said = execSync(`enkidu report ${state} ${named} "Fake Claude: made the change and checked it." --image ${shot}`, {
        encoding: "utf8",
        shell: "/bin/sh",
        env: { ...process.env, CLAUDECODE: "1" },
        timeout: 60_000,
      });
    } catch (e) {
      said = `${e.stdout ?? ""}${e.stderr ?? ""}`;
    }
    process.stdout.write(`${said.trim().split("\n").join("\r\n")}\r\n`);
  }
}

/** A command Claude runs (as its Bash tool would, marked CLAUDECODE=1): what it printed, as terminal lines. */
function sh(cmd) {
  let out = "";
  try {
    out = execSync(cmd, { encoding: "utf8", shell: "/bin/sh", env: { ...process.env, CLAUDECODE: "1" }, timeout: 60_000 });
  } catch (e) {
    out = `${e.stdout ?? ""}${e.stderr ?? ""}`;
  }
  return out.trim().split("\n").join("\r\n");
}

/** Claude's Bash tool: its PreToolUse hooks first (one may refuse the command, as Claude Code lets a hook do), then
 * the command. True when it ran. */
function bash(command) {
  const out = runHooks("PreToolUse", { tool_name: "Bash", tool_input: { command } }, "Bash");
  let answer = null;
  try {
    answer = out ? JSON.parse(out) : null;
  } catch {
    // no decision
  }
  if (answer?.hookSpecificOutput?.permissionDecision === "deny") {
    process.stdout.write(`Denied: ${answer.hookSpecificOutput.permissionDecisionReason}\r\n`);
    return false;
  }
  try {
    execSync(command, { cwd, shell: "/bin/sh", stdio: "ignore", timeout: 30_000 });
  } catch {
    // a failing command is Claude's to read
  }
  return true;
}

async function work(prompt) {
  await sleep(250);
  runHooks("PreToolUse", { tool_name: "Edit" }, "Edit");
  appendFileSync(join(cwd, "fake-output.txt"), `${prompt.split("\n")[0]}\n`);
  runHooks("PostToolUse", { tool_name: "Edit" }, "Edit");
  // the usage limit, as Claude Code 2.1 has it: what it shows, and the same words to the StopFailure hooks
  if (prompt.includes("FAKE_LIMIT")) {
    const said = "You've hit your limit · resets 3am (Europe/Paris)";
    process.stdout.write(`  ⎿  ${said}\r\n`);
    runHooks("StopFailure", { error: "rate_limit", last_assistant_message: said });
    return;
  }
  // a workflow's task (not the reminder to report): the fake tries to commit its work, as a Claude might (Enkidu
  // refuses it), looks where it stands in the workflow, and says how it goes with a screenshot
  if (prompt.includes("Enkidu's workflow") && !prompt.includes("you stopped without saying how")) {
    bash("git add -A && git commit -qm 'The fake commits its work'");
    process.stdout.write(`${sh("enkidu workflow")}\r\n`);
    const shot = join(cwd, "fake-half.png");
    writeFileSync(shot, Buffer.from(PNG, "base64"));
    process.stdout.write(`${sh(`enkidu update "Fake Claude: halfway there." --image ${shot}`)}\r\n`);
  }
  await sleep(250);
  process.stdout.write("Done.\r\n");
  stopped(prompt);
}

process.stdout.write(`\x1b[1mFake Claude\x1b[0m · session ${sessionId} · ${resumed ? "resumed" : "new"}\r\n`);
process.stdout.write(`args: ${argv.filter((a, i) => argv[i - 1] !== "--settings").join(" ")}\r\n`);
if (process.env.FAKE_GREETING) process.stdout.write(`${process.env.FAKE_GREETING}\r\n`);
// a screenful (or more) of what Claude Code prints, colors and all: the perf checks fill the terminal with it
if (process.env.FAKE_SCREEN) process.stdout.write(readFileSync(process.env.FAKE_SCREEN, "utf8"));
const started = runHooks("SessionStart", { source: resumed ? "resume" : "startup" });
// what a starting session is told (its PRs and tickets), shown like Claude Code adds it to the context
if (started) process.stdout.write(`[session] ${started.split("\n").join("\r\n")}\r\n`);

const rl = readline.createInterface({ input: process.stdin, terminal: false });
let waitingForPermission = null;
// A bracketed paste (ESC[200~ … ESC[201~) is one prompt, however many lines it has, like in real Claude Code.
let pasted = null;
const PASTE_START = "\x1b[200~";
const PASTE_END = "\x1b[201~";
process.stdout.write("> ");

// The last argument is a first message when it reads like one (real Claude takes `claude [flags] "prompt"`).
const last = argv.at(-1);
const initialPrompt = last && !last.startsWith("-") && /\s/.test(last) && !["--name", "--append-system-prompt"].includes(argv.at(-2)) ? last : null;
if (initialPrompt) {
  process.stdout.write(`${initialPrompt.split("\n")[0]}\r\n`);
  setTimeout(() => void submit(initialPrompt), 50);
}

async function submit(line) {
  saveTurn(line);
  const context = runHooks("UserPromptSubmit", { prompt: line });
  if (context) process.stdout.write(`[context] ${context}\r\n`);
  if (line === "ask") {
    waitingForPermission = line;
    runHooks("PermissionRequest", { tool_name: "Bash", tool_input: { command: "make test" } }, "Bash");
    process.stdout.write("Allow Bash(make test)? (y/n) ");
    return;
  }
  // `pr N`: Claude ran `gh pr create`, as Claude Code reports it to the hooks (the output and its gitOperation record)
  const pr = /^pr (\d+)$/.exec(line);
  if (pr) {
    const url = `https://github.com/${process.env.FAKE_REPO ?? "example-org/shop"}/pull/${pr[1]}`;
    runHooks("PreToolUse", { tool_name: "Bash", tool_input: { command: "gh pr create --fill" } }, "Bash");
    runHooks(
      "PostToolUse",
      {
        tool_name: "Bash",
        tool_input: { command: "gh pr create --fill", description: "Open the PR" },
        tool_response: { stdout: `${url}\n`, stderr: "", interrupted: false, isImage: false, gitOperation: { pr: { number: Number(pr[1]), url, action: "created" } } },
      },
      "Bash",
    );
    process.stdout.write(`${url}\r\n`);
    runHooks("Stop", { stop_hook_active: false });
    process.stdout.write("> ");
    return;
  }
  // `enkidu …`: Claude runs Enkidu's command line, in its own environment (as its Bash tool would)
  if (line.startsWith("enkidu ")) {
    let out = "";
    try {
      // Claude Code marks the commands it runs with CLAUDECODE=1
      out = execSync(line, { encoding: "utf8", shell: "/bin/sh", env: { ...process.env, CLAUDECODE: "1" }, timeout: 120_000 });
    } catch (e) {
      out = `${e.stdout ?? ""}${e.stderr ?? ""}`;
    }
    process.stdout.write(`${out.split("\n").join("\r\n")}\r\n> `);
    return;
  }
  // `push BRANCH`: Claude pushed a branch
  const push = /^push (\S+)$/.exec(line);
  if (push) {
    runHooks(
      "PostToolUse",
      { tool_name: "Bash", tool_input: { command: `git push -u origin ${push[1]}` }, tool_response: { stdout: "", stderr: "", interrupted: false, isImage: false, gitOperation: { push: { branch: push[1] } } } },
      "Bash",
    );
    runHooks("Stop", { stop_hook_active: false });
    process.stdout.write("pushed\r\n> ");
    return;
  }
  const spin = /^spin (\d+(?:\.\d+)?)$/.exec(line);
  if (spin) {
    const frames = ["·", "✢", "✳", "✶", "✻", "✽"];
    const until = Date.now() + Number(spin[1]) * 1000;
    for (let i = 0; Date.now() < until; i++) {
      process.stdout.write(`\r${frames[i % frames.length]} Working… (esc to interrupt)`);
      await sleep(100);
    }
    process.stdout.write("\r\n  ⎿  Interrupted · What should Claude do instead?\r\n> ");
    return;
  }
  await work(line);
  process.stdout.write("> ");
}

rl.on("line", async (input) => {
  // Esc typed while it "worked" waits in the terminal's line buffer and comes in front of the next line; real Claude
  // reads keys one by one, so here it is just dropped (bracketed-paste markers stay)
  let raw = input.replace(/\x1b(?!\[20[01]~)/g, "");
  if (pasted === null && raw.includes(PASTE_START)) {
    pasted = [];
    raw = raw.slice(raw.indexOf(PASTE_START) + PASTE_START.length);
  }
  if (pasted !== null) {
    if (!raw.includes(PASTE_END)) {
      pasted.push(raw);
      return;
    }
    pasted.push(raw.slice(0, raw.indexOf(PASTE_END)));
    const text = pasted.join("\n");
    pasted = null;
    process.stdout.write(`[Pasted text +${text.split("\n").length} lines]\r\n`);
    await submit(text);
    return;
  }
  const line = raw.trim();
  if (waitingForPermission) {
    const prompt = waitingForPermission;
    waitingForPermission = null;
    if (line === "y") {
      runHooks("PostToolUse", { tool_name: "Bash" }, "Bash");
      await work(prompt);
    } else {
      process.stdout.write("Denied.\r\n");
      runHooks("Stop", { stop_hook_active: false });
    }
    process.stdout.write("> ");
    return;
  }
  if (!line) {
    process.stdout.write("> ");
    return;
  }
  if (line === "exit") {
    runHooks("SessionEnd", { reason: "prompt_input_exit" });
    process.exit(0);
  }
  // `mouse on` / `mouse off`: Claude Code in full screen takes the mouse (SGR reports), so the wheel comes in as input.
  // (The reports wait in the terminal's line buffer and come in front of the next line, so only the end is matched.)
  const mouse = /mouse (on|off)$/.exec(line);
  if (mouse) {
    process.stdout.write(mouse[1] === "on" ? "\x1b[?1000h\x1b[?1006h" : "\x1b[?1000l\x1b[?1006l");
    process.stdout.write(`Mouse reporting ${mouse[1]}\r\n> `);
    return;
  }
  // `/resume ID [FOLDER]`: Claude Code's /resume picking another conversation. The one it was in ends and the other
  // starts as "resume"; one saved for another folder (the main folder, say) moves Claude into that folder, as Claude
  // Code does with a conversation from another worktree of the same repo.
  if (line.startsWith("/resume ")) {
    const [, id, folder] = line.split(/\s+/);
    runHooks("SessionEnd", { reason: "other" });
    sessionId = id;
    if (folder) {
      process.chdir(folder);
      cwd = folder;
    }
    projectDir = join(configDir, "projects", cwd.replace(/[^A-Za-z0-9]/g, "-"));
    transcript = join(projectDir, `${sessionId}.jsonl`);
    const context = runHooks("SessionStart", { source: "resume" });
    if (context) process.stdout.write(`[session] ${context.split("\n").join("\r\n")}\r\n`);
    process.stdout.write(`Resumed ${id}\r\n> `);
    return;
  }
  await submit(line);
});
rl.on("close", () => {
  runHooks("SessionEnd", { reason: "other" });
  process.exit(0);
});

if (process.env.FAKE_CLAUDE_TOUCH) writeFileSync(join(cwd, process.env.FAKE_CLAUDE_TOUCH), "started\n");
if (existsSync(transcript) && process.env.FAKE_CLAUDE_LOG) {
  // nothing else: the transcript only proves a session can be resumed
}
