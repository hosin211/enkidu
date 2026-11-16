#!/usr/bin/env node
// A fake language server for tests. It speaks the protocol over its standard input and output as a real one does,
// reads nothing of any project, and logs every message it gets (the method, and what the test looks at) to
// FAKE_LSP_LOG, one JSON object a line.
//
// What it answers to "where is this defined" is read on every question from FAKE_LSP_ANSWER (a file holding the
// JSON of the answer: null, a place, or a list of places). The mode is read on every message from FAKE_LSP_MODE
// (a file), so a test can break the server and mend it while the core runs:
//   ok        answers at once
//   slow      answers a definition after a second (past the core's limit)
//   silent    never answers a definition
//   dead      ends as soon as it is started
//   mute      says hello without saying that it finds definitions
//   dies      ends when it is asked for a definition
//   asks      asks the client for its settings and to watch files right after hello, as real servers do
import { appendFileSync, existsSync, readFileSync } from "node:fs";

const logFile = process.env.FAKE_LSP_LOG;
const modeFile = process.env.FAKE_LSP_MODE;
const answerFile = process.env.FAKE_LSP_ANSWER;
const mode = () => (modeFile && existsSync(modeFile) ? readFileSync(modeFile, "utf8").trim() : "") || "ok";
const log = (entry) => logFile && appendFileSync(logFile, JSON.stringify(entry) + "\n");

if (mode() === "dead") process.exit(3);
// the folder it was started in, and whether the app's own variables reached it (they must not)
log({ started: process.cwd(), pid: process.pid, own: Object.keys(process.env).filter((name) => /^ENKIDU_/.test(name)) });

const send = (message) => {
  const body = JSON.stringify({ jsonrpc: "2.0", ...message });
  process.stdout.write(`Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
};

let asked = 0;
function got(message) {
  const { id, method, params } = message;
  if (method === undefined) {
    // an answer to something this server asked
    log({ answered: id, result: message.result });
    return;
  }
  switch (method) {
    case "initialize":
      log({ method, root: params.rootUri, watches: params.capabilities?.workspace?.didChangeWatchedFiles });
      send({ id, result: { capabilities: mode() === "mute" ? {} : { definitionProvider: true } } });
      break;
    case "initialized":
      log({ method });
      if (mode() === "asks") {
        send({ id: `s${++asked}`, method: "workspace/configuration", params: { items: [{ section: "a" }, { section: "b" }] } });
        send({ id: `s${++asked}`, method: "client/registerCapability", params: { registrations: [] } });
      }
      break;
    case "textDocument/didOpen":
      log({ method, uri: params.textDocument.uri, language: params.textDocument.languageId, text: params.textDocument.text });
      break;
    case "textDocument/didClose":
      log({ method, uri: params.textDocument.uri });
      break;
    case "workspace/didChangeWatchedFiles":
      log({ method, changes: params.changes });
      break;
    case "textDocument/definition": {
      log({ method, uri: params.textDocument.uri, position: params.position });
      const result = answerFile && existsSync(answerFile) ? JSON.parse(readFileSync(answerFile, "utf8")) : null;
      const now = mode();
      if (now === "silent") break;
      if (now === "dies") process.exit(1);
      if (now === "slow") setTimeout(() => send({ id, result }), 1000);
      else send({ id, result });
      break;
    }
    case "shutdown":
      send({ id, result: null });
      break;
    case "exit":
      process.exit(0);
      break;
    default:
      log({ method });
      if (id !== undefined) send({ id, result: null });
  }
}

let pending = Buffer.alloc(0);
process.stdin.on("data", (chunk) => {
  pending = Buffer.concat([pending, chunk]);
  for (;;) {
    const end = pending.indexOf("\r\n\r\n");
    if (end < 0) return;
    const length = Number(/Content-Length: *(\d+)/i.exec(pending.subarray(0, end).toString())?.[1] ?? 0);
    if (pending.length < end + 4 + length) return;
    const body = pending.subarray(end + 4, end + 4 + length).toString();
    pending = pending.subarray(end + 4 + length);
    try {
      got(JSON.parse(body));
    } catch (error) {
      log({ garbage: String(error) });
    }
  }
});
// its input closed: the app is gone, or gave it up
process.stdin.on("end", () => process.exit(0));
