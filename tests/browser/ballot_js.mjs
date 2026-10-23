// Browser check of the voting page's script (static/js/, native ES
// modules) in headless Chromium, driven over the DevTools protocol.
// Dependency-free: Node (with its built-in fetch and WebSocket) and Chromium
// only. Not part of `cargo test`; run it by hand after changing the script,
// its stylesheet or the markup it relies on:
//
//   cargo build
//   nix shell nixpkgs#nodejs nixpkgs#chromium -c node tests/browser/ballot_js.mjs
//   BASE_PATH=/polls/ nix shell nixpkgs#nodejs nixpkgs#chromium -c node tests/browser/ballot_js.mjs
//
// The second run serves the app under a sub-path (`public_base_url` ending
// in `/polls/`, every request under `/polls/…`, as a reverse proxy that
// forwards the prefix sends them) and also checks that the page requests
// nothing outside it. Run it both ways after changing the script.
//
// It starts the debug binary (`target/debug/vota`, or $VOTA_BIN) on a free
// local port against a temp database (with a folder Theme whose template
// has no countdown placeholder), creates Ballots through the admin API, and
// checks with JavaScript enabled: the short local-time deadline with its
// precise tooltip and the countdown placeholder; Editable autosave (no
// Submit button, saved/pending/failed feedback per Option and in the status
// line, strictly one request in flight with coalescing of fast clicks, the
// multiple-choice debounce, automatic retries with backoff, Retry-After,
// "Retry now" and the online event, definitive errors not retried, the 409
// dialogs and reloads, the beforeunload prompt only while something is not
// stored); autosave and the Final confirmation starting from a
// Preselection; the Final confirmation dialog and its error dialogs; an Option
// the admin replaced or a title the admin changed while the page was open
// (409 `ballot_changed`, then a reload; the script sends the page's
// revision); the countdown closing the page, with and without a
// placeholder; focus and reduced motion; an accent-colour Ballot's in-place
// update (its nonce'd style); the modules loading under the CSP from their
// content-versioned directory, no requests to other hosts and no CSP
// violations; and, with JavaScript disabled, the plain form flows (Final
// and Editable), the replaced-Option notice and a title changed between the
// confirmation step and confirming.

import { spawn, execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createServer } from "node:net";

const BIN = process.env.VOTA_BIN ?? "target/debug/vota";
const CHROMIUM = process.env.CHROMIUM ?? "chromium";
// The base path (`/`, or e.g. `/polls/`), with leading and trailing slash.
const BASE_PATH = `/${(process.env.BASE_PATH ?? "/").replace(/^\/+|\/+$/g, "")}/`.replace("//", "/");
// The base path without its trailing slash: "" at the root.
const PREFIX = BASE_PATH.slice(0, -1);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

let failures = 0;
function check(condition, what, detail) {
  if (condition) {
    console.log(`ok   ${what}`);
  } else {
    failures++;
    console.log(`FAIL ${what}${detail === undefined ? "" : `: ${JSON.stringify(detail)}`}`);
  }
}

async function freePort() {
  return new Promise((resolve) => {
    const server = createServer();
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

async function waitFor(what, probe, timeout = 10000) {
  const until = Date.now() + timeout;
  for (;;) {
    const value = await probe();
    if (value) return value;
    if (Date.now() > until) throw new Error(`timed out waiting for ${what}`);
    await sleep(50);
  }
}

// --- The server ------------------------------------------------------------

const dir = mkdtempSync(join(tmpdir(), "vota-browser-"));
// A folder Theme whose voting page has no countdown placeholder.
const themes = join(dir, "themes");
mkdirSync(join(themes, "bare", "templates"), { recursive: true });
writeFileSync(join(themes, "bare", "style.css"), "");
writeFileSync(
  join(themes, "bare", "templates", "ballot.html"),
  readFileSync("templates/ballot.html", "utf8").replace('<span class="vota-countdown" role="timer"></span>', ""),
);
const port = await freePort();
const origin = `http://127.0.0.1:${port}`;
// Where the app's routes are: the origin plus the prefix.
const root = `${origin}${PREFIX}`;
const env = {
  ...process.env,
  VOTA_BIND_ADDRESS: "127.0.0.1",
  VOTA_PORT: String(port),
  VOTA_DATABASE_PATH: join(dir, "vota.db"),
  VOTA_PUBLIC_BASE_URL: `${origin}${BASE_PATH}`,
  VOTA_DEFAULT_TIME_ZONE: "Europe/Berlin",
  VOTA_THEME_FOLDER: themes,
};
const adminToken = execFileSync(BIN, ["admin-token", "create", "--label", "browser-check"], { env })
  .toString()
  .trim()
  .split(/\s+/)
  .find((word) => word.startsWith("vta_"));
const server = spawn(BIN, ["serve"], { env, stdio: ["ignore", "ignore", "inherit"] });
await waitFor("the server", () => fetch(`${root}/healthz`).then((r) => r.ok, () => false));

async function createBallot(fields) {
  const response = await fetch(`${root}/admin/ballots`, {
    method: "POST",
    headers: { authorization: `Bearer ${adminToken}`, "content-type": "application/json" },
    body: JSON.stringify({
      title: "Lunch on Friday?",
      options: [{ label: "Pizza" }, { label: "Sushi" }, { label: "Tacos" }],
      voting_deadline: new Date(Date.now() + 3 * 86400e3).toISOString(),
      ...fields,
    }),
  });
  if (response.status !== 201) throw new Error(`create: ${response.status} ${await response.text()}`);
  return response.json();
}

/** The admin replaces the Ballot's Options (which gives them new IDs). */
async function replaceOptions(ballot, labels) {
  const response = await fetch(`${root}/admin/ballots/${ballot.id}`, {
    method: "PATCH",
    headers: { authorization: `Bearer ${adminToken}`, "content-type": "application/json" },
    body: JSON.stringify({ options: labels.map((label) => ({ label })) }),
  });
  if (response.status !== 200) throw new Error(`edit: ${response.status} ${await response.text()}`);
  return response.json();
}

/** The admin edits the Ballot's title (its Options stay as they are). */
async function retitle(ballot, title) {
  const response = await fetch(`${root}/admin/ballots/${ballot.id}`, {
    method: "PATCH",
    headers: { authorization: `Bearer ${adminToken}`, "content-type": "application/json" },
    body: JSON.stringify({ title }),
  });
  if (response.status !== 200) throw new Error(`edit: ${response.status} ${await response.text()}`);
  return response.json();
}

const voterView = (token) => fetch(`${root}/api/b/${token}`).then((r) => r.json());

// --- The browser -----------------------------------------------------------

const profile = join(dir, "profile");
const browser = spawn(
  CHROMIUM,
  [
    "--headless=new",
    "--no-sandbox",
    "--no-first-run",
    "--disable-gpu",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    "about:blank",
  ],
  { stdio: ["ignore", "ignore", "pipe"] },
);
const browserUrl = await new Promise((resolve, reject) => {
  let seen = "";
  browser.stderr.on("data", (chunk) => {
    seen += chunk;
    const match = seen.match(/DevTools listening on (ws:\/\/\S+)/);
    if (match) resolve(match[1]);
  });
  browser.on("exit", () => reject(new Error(`chromium exited: ${seen}`)));
});
const devtools = new URL(browserUrl);
const targets = await fetch(`http://${devtools.host}/json/list`).then((r) => r.json());
const pageTarget = targets.find((t) => t.type === "page");

const socket = new WebSocket(pageTarget.webSocketDebuggerUrl);
await new Promise((resolve) => socket.addEventListener("open", resolve, { once: true }));
let nextId = 1;
const pending = new Map();
const listeners = [];
socket.addEventListener("message", (event) => {
  const message = JSON.parse(event.data);
  if (message.id && pending.has(message.id)) {
    const { resolve, reject } = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) reject(new Error(JSON.stringify(message.error)));
    else resolve(message.result);
  } else if (message.method) {
    for (const listener of listeners) listener(message);
  }
});
function send(method, params = {}) {
  const id = nextId++;
  socket.send(JSON.stringify({ id, method, params }));
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
}

// Everything the page requests, and every console error or CSP violation.
const requested = [];
const problems = [];
// Voter API calls to answer with a canned response (or fail, or hold until
// released) instead: null (pass), "network", "hold", or { status, body,
// headers }.
let intercept = null;
// Every voter API PUT the page sent, in order: { selection, at }.
const puts = [];
// Requests held back ("hold"), in order: { requestId, selection }.
const held = [];
// JavaScript dialogs the browser opened (e.g. beforeunload).
const jsDialogs = [];
let stayOnBeforeUnload = false;
listeners.push(async (message) => {
  const { method, params } = message;
  if (method === "Network.requestWillBeSent") requested.push(params.request.url);
  if (method === "Runtime.exceptionThrown") problems.push(params.exceptionDetails.text);
  if (method === "Runtime.consoleAPICalled" && params.type === "error") {
    problems.push(params.args.map((a) => a.value ?? a.description).join(" "));
  }
  // (The browser logs every non-2xx response as an error; those are
  // expected here: the error answers are forced on purpose.)
  if (
    method === "Log.entryAdded" &&
    params.entry.level === "error" &&
    !params.entry.text.startsWith("Failed to load resource")
  ) {
    problems.push(params.entry.text);
  }
  if (method === "Page.javascriptDialogOpening") {
    jsDialogs.push(params.type);
    await send("Page.handleJavaScriptDialog", { accept: !(params.type === "beforeunload" && stayOnBeforeUnload) });
  }
  if (method === "Fetch.requestPaused") {
    const canned = intercept;
    const selection = JSON.parse(params.request.postData ?? "{}").option_ids ?? null;
    puts.push({ selection, at: Date.now() });
    if (canned === "hold") {
      held.push({ requestId: params.requestId, selection });
    } else if (!canned) {
      await send("Fetch.continueRequest", { requestId: params.requestId });
    } else if (canned === "network") {
      await send("Fetch.failRequest", { requestId: params.requestId, errorReason: "InternetDisconnected" });
    } else {
      await send("Fetch.fulfillRequest", {
        requestId: params.requestId,
        responseCode: canned.status,
        responseHeaders: [{ name: "content-type", value: "application/json" }, ...(canned.headers ?? [])],
        body: Buffer.from(JSON.stringify(canned.body)).toString("base64"),
      });
    }
  }
});
await send("Page.enable");
await send("Runtime.enable");
await send("Network.enable");
await send("Log.enable");
await send("Fetch.enable", { patterns: [{ urlPattern: "*/api/b/*/response" }] });
// The voter is in New York; the server's configured zone is Berlin.
await send("Emulation.setTimezoneOverride", { timezoneId: "America/New_York" });

async function evaluate(expression) {
  // As a user gesture: the page gets sticky activation, which browsers
  // require before they show a beforeunload prompt.
  const result = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true, userGesture: true });
  if (result.exceptionDetails) throw new Error(`${expression}: ${result.exceptionDetails.text}`);
  return result.result.value;
}

function waitForLoad() {
  return new Promise((resolve) => {
    const listener = (message) => {
      if (message.method === "Page.loadEventFired") {
        listeners.splice(listeners.indexOf(listener), 1);
        resolve();
      }
    };
    listeners.push(listener);
  });
}

async function open(path) {
  const loaded = waitForLoad();
  await send("Page.navigate", { url: `${root}${path}` });
  await loaded;
  // Marks this document: gone after a reload, kept by in-place updates.
  await evaluate("window.__sameDocument = true");
}

const dialogState = () => evaluate(`document.querySelector("dialog.vota-dialog[open]")?.dataset.state ?? null`);
const dialogText = () => evaluate(`document.querySelector("dialog.vota-dialog")?.innerText ?? ""`);
const waitForDialog = (state) => waitFor(`dialog ${state}`, async () => (await dialogState()) === state);
const click = (selector) => evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
const choose = (label) =>
  evaluate(`[...document.querySelectorAll("label")].find(l => l.textContent.trim() === ${JSON.stringify(label)}).click()`);
const clickButton = (text) =>
  evaluate(`[...document.querySelectorAll("button")].find(b => b.textContent.trim() === ${JSON.stringify(text)}).click()`);
const sameDocument = () => evaluate("window.__sameDocument === true");
/** The autosave status line: [data-state, its text]. */
const autosaveStatus = () =>
  evaluate(`(() => { const b = document.querySelector(".vota-autosave"); return b ? [b.dataset.state, b.querySelector("[role=status]").textContent] : null; })()`);
const waitForAutosave = (state, timeout) =>
  waitFor(`autosave ${state}`, async () => (await autosaveStatus())?.[0] === state, timeout);
/** The save state of the card of the Option with this label. */
const cardState = (label) =>
  evaluate(`[...document.querySelectorAll("label")].find(l => l.textContent.trim() === ${JSON.stringify(label)}).closest(".option").dataset.votaSave ?? null`);
/** Lets the held requests through, oldest first. */
async function release() {
  while (held.length) {
    await send("Fetch.continueRequest", { requestId: held.shift().requestId });
  }
}
const checkedLabels = () =>
  evaluate(`[...document.querySelectorAll("input[name=option]:checked")].map(i => i.labels[0].textContent.trim()).join(",")`);
const storedLabels = async (ballot) => {
  const view = await voterView(ballot.token);
  return (view.response?.option_ids ?? []).map((id) => ballot.options.find((o) => o.id === id).label).join(",");
};
/** Whether leaving now would ask first (a synthetic beforeunload). */
const wouldWarn = () =>
  evaluate(`(() => { const e = new Event("beforeunload", { cancelable: true }); window.dispatchEvent(e); return e.defaultPrevented; })()`);
const visible = (selector) => evaluate(`(() => { const n = document.querySelector(${JSON.stringify(selector)}); return !!n && n.getClientRects().length > 0; })()`);
const waitForReload = () => waitFor("a reload", async () => (await evaluate("window.__sameDocument")) !== true);

try {
  // --- Editable: local time, countdown, autosave ---------------------------
  {
    const ballot = await createBallot({ submission_mode: "editable", theme: "dark" });
    await open(`/b/${ballot.token}`);
    const time = await evaluate(`document.querySelector("time").textContent`);
    check(
      /^(Today|Tomorrow|(Mon|Tue|Wed|Thu|Fri|Sat|Sun) \d{1,2} [A-Z][a-z]{2}( \d{4})?), \d\d:\d\d$/.test(time),
      "the deadline shows short and friendly, without a zone",
      time,
    );
    const title = await evaluate(`document.querySelector("time").title`);
    check(/(EST|EDT|GMT-[45])/.test(title), "the precise local deadline, with its zone, is the tooltip", title);
    check(
      (await evaluate(`document.querySelector("time").getAttribute("datetime")`)) === ballot.voting_deadline.replace(/\.\d+Z$/, "Z") ||
        Date.parse(await evaluate(`document.querySelector("time").getAttribute("datetime")`)) === Date.parse(ballot.voting_deadline),
      "the full instant stays on <time datetime>",
    );
    const countdown = await evaluate(`document.querySelector(".vota-countdown").textContent`);
    check(/^in (2 days, 23 hours|3 days, 0 hours)$/.test(countdown), "the countdown fills the template's placeholder", countdown);
    check(
      (await evaluate(`document.querySelectorAll(".vota-countdown").length`)) === 1,
      "the script adds no countdown of its own",
    );
    check(!(await visible('button[type="submit"]')), "an Editable Ballot shows no Submit button");
    check(
      (await evaluate(`document.querySelectorAll('button[type="submit"]').length`)) === 1,
      "the Submit button stays in the page for the no-JS flow",
    );
    check((await autosaveStatus())?.[0] === "idle", "the status line says changes are saved automatically", await autosaveStatus());

    const before = puts.length;
    await choose("Sushi");
    check(
      (await cardState("Sushi")) === "pending" || (await cardState("Sushi")) === "saved",
      "the chosen Option is marked at once",
      await cardState("Sushi"),
    );
    await waitForAutosave("saved");
    check((await autosaveStatus())[1].includes("Saved"), "the status line says Saved", await autosaveStatus());
    check((await cardState("Sushi")) === "saved", "the saved Option is highlighted as saved", await cardState("Sushi"));
    check(await dialogState() === null, "autosave opens no dialog");
    check(puts.length - before === 1, "one change, one request", puts.slice(before));
    check((await storedLabels(ballot)) === "Sushi", "the Response was recorded through the voter API");
    await waitFor("the refreshed view", () => evaluate(`document.querySelector(".status")?.dataset.state === "answered"`));
    check(await sameDocument(), "after the first save the page updates in place, without a reload");
    check((await autosaveStatus())?.[0] === "saved", "the refreshed page still says Saved", await autosaveStatus());
    check(!!(await evaluate(`document.querySelector(".vota-countdown").textContent`)), "the countdown is back after the update");
    await sleep(2300);
    check((await cardState("Sushi")) === null, "the saved highlight fades", await cardState("Sushi"));
    check(!(await wouldWarn()), "with everything saved, leaving asks nothing");

    // Changing the answer works again in the same document.
    await choose("Tacos");
    await waitForAutosave("saved");
    await waitFor("stored", async () => (await storedLabels(ballot)) === "Tacos");
    check(true, "a second change is saved too");
    check(await sameDocument(), "later saves keep the page");
  }

  // --- Fast clicks: strictly serialised, coalesced ---------------------------
  {
    const ballot = await createBallot({ submission_mode: "editable" });
    await open(`/b/${ballot.token}`);
    const before = puts.length;
    intercept = "hold";
    await choose("Pizza");
    await waitFor("the first request", () => held.length === 1);
    await choose("Sushi");
    await choose("Tacos");
    await sleep(300);
    check(held.length === 1 && puts.length - before === 1, "while a save is in flight nothing else is sent", puts.slice(before));
    check((await autosaveStatus())[0] === "pending", "the status line says Saving…", await autosaveStatus());
    check(
      (await cardState("Tacos")) === "pending" && (await checkedLabels()) === "Tacos",
      "the newest click shows at once, as pending",
    );
    check(await wouldWarn(), "while a save is pending, leaving asks first");
    await release();
    await waitFor("the next request", () => held.length === 1);
    check(
      JSON.stringify(puts.slice(before).map((p) => p.selection)) ===
        JSON.stringify([[ballot.options[0].id], [ballot.options[2].id]]),
      "the next request carries only the latest selection, after the first finished",
      puts.slice(before),
    );
    check((await autosaveStatus())[0] === "pending", "the first success doesn't say Saved while newer changes are pending");
    intercept = null;
    await release();
    await waitForAutosave("saved");
    check((await storedLabels(ballot)) === (await checkedLabels()), "the stored Response is what the page shows");
    check(puts.length - before === 2, "two requests in all", puts.slice(before));
  }

  // --- Multiple choice: rapid clicks are debounced ------------------------------
  {
    const ballot = await createBallot({ submission_mode: "editable", choice_mode: "multiple" });
    await open(`/b/${ballot.token}`);
    const before = puts.length;
    await choose("Pizza");
    await sleep(100);
    await choose("Tacos");
    await waitForAutosave("saved");
    check(
      puts.length - before === 1 && puts[before].selection.length === 2,
      "clicks within the debounce go out as one request",
      puts.slice(before),
    );
    check((await storedLabels(ballot)) === "Pizza,Tacos", "both Options are stored");
    // Unticking everything can't be sent: the page says so, and keeps the
    // stored answer.
    const sent = puts.length;
    await choose("Pizza");
    await choose("Tacos");
    await waitForAutosave("invalid");
    await sleep(600);
    check(puts.length === sent, "an empty multiple-choice selection is not sent");
    check((await cardState("Pizza")) === "failed", "the unticked Options show as not saved", await cardState("Pizza"));
    check(await wouldWarn(), "an unsaved selection makes leaving ask first");
    await choose("Tacos");
    await waitForAutosave("saved");
    check((await storedLabels(ballot)) === "Tacos", "choosing again saves");
  }

  // --- Allow No Option: unticking everything saves "no Option" ------------------
  {
    const ballot = await createBallot({ submission_mode: "editable", choice_mode: "multiple", allow_no_option: true });
    await open(`/b/${ballot.token}`);
    check(
      (await evaluate(`document.querySelector("legend").textContent.trim()`)) === "Choose any number of options (or none)",
      "the legend says no Option is fine",
    );
    check(await visible('button[type="submit"]'), "without an answer, Save stays to save no Option");
    await choose("Pizza");
    await waitForAutosave("saved");
    const before = puts.length;
    await choose("Pizza");
    await waitForAutosave("saved");
    check(
      puts.length - before === 1 && puts[before].selection.length === 0,
      "unticking everything sends the empty selection",
      puts.slice(before),
    );
    const view = await voterView(ballot.token);
    check(
      view.status === "answered" && view.response?.option_ids.length === 0,
      "the empty Response is stored",
      view,
    );
    check(!(await wouldWarn()), "a saved empty selection leaves without asking");
    check((await cardState("Pizza")) !== "failed", "the unticked Option is not shown as unsaved");

    // A fresh Ballot: Save with nothing ticked records "no Option".
    const fresh = await createBallot({ submission_mode: "editable", choice_mode: "multiple", allow_no_option: true });
    await open(`/b/${fresh.token}`);
    check(!(await wouldWarn()), "an untouched page has nothing unsaved");
    check((await autosaveStatus())?.[0] === "idle", "an untouched page is idle", await autosaveStatus());
    const sent = puts.length;
    await click('button[type="submit"]');
    await waitForAutosave("saved");
    check(puts.length - sent === 1 && puts[sent].selection.length === 0, "Save sends the empty selection", puts.slice(sent));
    check((await voterView(fresh.token)).status === "answered", "Save with nothing ticked answers the Ballot");
    await waitFor("the page refreshed in place", async () => (await evaluate(`document.querySelector(".status")?.dataset.state`)) === "answered");
    check(!(await visible('button[type="submit"]')), "once answered, Save is hidden again");
    check(await sameDocument(), "the page was refreshed in place, not reloaded");
  }

  // --- Without Allow No Option, ticking and unticking at once is no answer ----------
  {
    const ballot = await createBallot({ submission_mode: "editable", choice_mode: "multiple" });
    await open(`/b/${ballot.token}`);
    check(!(await visible('button[type="submit"]')), "without Allow No Option, Save is hidden from the start");
    const before = puts.length;
    await choose("Pizza");
    await choose("Pizza");
    await sleep(600);
    check(puts.length === before, "an empty selection without a Response sends nothing", puts.slice(before));
    check((await autosaveStatus())?.[0] === "idle", "and the page stays idle", await autosaveStatus());
    check(!(await wouldWarn()), "and leaving doesn't ask");
    check((await voterView(ballot.token)).status === "unanswered", "the Ballot stays unanswered");
  }

  // --- Allow No Option, Final: the dialog confirms "no Option" ---------------------
  {
    const ballot = await createBallot({ choice_mode: "multiple", submission_mode: "final", allow_no_option: true });
    await open(`/b/${ballot.token}`);
    await click('button[type="submit"]');
    await waitForDialog("confirm");
    check((await dialogText()).includes("You chose no option"), "the dialog confirms no Option", await dialogText());
    await clickButton("Confirm and submit");
    await waitForDialog("saved");
    const view = await voterView(ballot.token);
    check(view.status === "answered" && view.response?.option_ids.length === 0, "the empty Final Response is stored", view);
    await clickButton("OK");
    await waitFor("the read-only view", async () => (await evaluate(`document.body.innerText`)).includes("You chose no option."));
  }

  // --- Preselection, Editable: autosave starts from the preselected Response ----------
  {
    const ballot = await createBallot({ submission_mode: "editable", choice_mode: "multiple", preselection: ["Sushi", "Tacos"] });
    await open(`/b/${ballot.token}`);
    check((await checkedLabels()) === "Sushi,Tacos", "the preselected Options are ticked", await checkedLabels());
    check(
      (await evaluate(`document.body.innerText`)).includes("Preselected for you. You can change it until voting closes."),
      "the page says the answer is preselected",
    );
    check((await autosaveStatus())?.[0] === "idle", "the preselected Response counts as stored", await autosaveStatus());
    check(!(await wouldWarn()), "the preselected Response leaves without asking");
    check(await visible('button[type="submit"]'), "while preselected, Save stays to keep it as the voter's");
    const before = puts.length;
    await click('button[type="submit"]');
    await waitForAutosave("saved");
    check(
      puts.length - before === 1 &&
        JSON.stringify(puts[before].selection) === JSON.stringify([ballot.options[1].id, ballot.options[2].id]),
      "Save sends the preselected Options unchanged",
      puts.slice(before),
    );
    check((await voterView(ballot.token)).response?.origin === "voter", "saving them makes the Response the voter's");
    await waitFor("the page refreshed in place", async () => !(await evaluate(`document.body.innerText`)).includes("Preselected"));
    check(await sameDocument(), "the notice goes without a reload");
    check(!(await visible('button[type="submit"]')), "once the voter's, Save is hidden again");

    // A change from the preselected state is saved like any other.
    const fresh = await createBallot({ submission_mode: "editable", choice_mode: "multiple", preselection: ["Sushi"] });
    await open(`/b/${fresh.token}`);
    const sent = puts.length;
    await choose("Pizza");
    await waitForAutosave("saved");
    check(
      puts.length - sent === 1 &&
        JSON.stringify(puts[sent].selection) === JSON.stringify([fresh.options[0].id, fresh.options[1].id]),
      "a change sends the preselected Options with the new one",
      puts.slice(sent),
    );
    const view = await voterView(fresh.token);
    check(view.response?.origin === "voter" && (await storedLabels(fresh)) === "Pizza,Sushi", "the change is stored as the voter's", view);
    await waitFor("the page refreshed in place", async () => !(await evaluate(`document.body.innerText`)).includes("Preselected"));
    check(await sameDocument(), "the page updates in place after the change");
  }

  // --- Preselection, Final: ticked, then confirmed as ever ---------------------------
  {
    const ballot = await createBallot({ submission_mode: "final", preselection: ["Tacos"] });
    await open(`/b/${ballot.token}`);
    check((await checkedLabels()) === "Tacos", "the preselected Option is ticked", await checkedLabels());
    check(
      (await evaluate(`document.body.innerText`)).includes("Preselected; confirm or change, then submit."),
      "the page asks to confirm or change the preselection",
    );
    check((await voterView(ballot.token)).status === "unanswered", "nothing is recorded before the voter submits");
    await click('button[type="submit"]');
    await waitForDialog("confirm");
    check((await dialogText()).includes("Tacos"), "the dialog confirms the preselected Option", await dialogText());
    await clickButton("Confirm and submit");
    await waitForDialog("saved");
    const view = await voterView(ballot.token);
    check(view.status === "answered" && view.response?.origin === "voter", "confirming records the voter's Response", view);
    await clickButton("OK");
    await waitFor("the read-only view", () => evaluate(`!document.querySelector("form")`));
  }

  // --- Transient failures are retried ---------------------------------------------
  {
    const ballot = await createBallot({ submission_mode: "editable" });
    await open(`/b/${ballot.token}`);
    const before = puts.length;
    intercept = { status: 500, body: { error: "internal" } };
    await choose("Pizza");
    await waitForAutosave("failed");
    const status = await autosaveStatus();
    check(status[1].includes("Not saved, retrying…"), "a 500 says Not saved, retrying…", status);
    check((await cardState("Pizza")) === "failed", "the Option shows as not saved", await cardState("Pizza"));
    check(await visible(".vota-retry"), "Retry now is offered");
    check(await wouldWarn(), "while a save failed, leaving asks first");
    await waitFor("an automatic retry", () => puts.length - before >= 2, 3000);
    check(puts[before + 1].at - puts[before].at >= 900, "the first retry waits about a second", puts[before + 1].at - puts[before].at);
    intercept = null;
    await waitForAutosave("saved", 6000);
    check((await storedLabels(ballot)) === "Pizza", "once the server is back, the retry saves");
    check((await cardState("Pizza")) === "saved", "and the Option turns saved");
    check(!(await wouldWarn()), "then leaving asks nothing");
  }
  {
    // 429 with Retry-After: the retry waits; coming back online retries now.
    const ballot = await createBallot({ submission_mode: "editable" });
    await open(`/b/${ballot.token}`);
    const before = puts.length;
    intercept = { status: 429, body: { error: "too_many_requests" }, headers: [{ name: "Retry-After", value: "30" }] };
    await choose("Sushi");
    await waitForAutosave("failed");
    check((await autosaveStatus())[1].includes("Too many requests"), "429: says why", await autosaveStatus());
    await sleep(2500);
    check(puts.length - before === 1, "429: the retry honours Retry-After", puts.slice(before));
    intercept = null;
    await evaluate(`window.dispatchEvent(new Event("online"))`);
    await waitForAutosave("saved", 3000);
    check((await storedLabels(ballot)) === "Sushi", "coming back online retries at once");
  }
  {
    // A change while a retry waits sends the newest state.
    const ballot = await createBallot({ submission_mode: "editable" });
    await open(`/b/${ballot.token}`);
    const before = puts.length;
    intercept = "network";
    await choose("Pizza");
    await waitForAutosave("failed");
    check((await autosaveStatus())[1].includes("No connection"), "a network error says so", await autosaveStatus());
    intercept = null;
    await choose("Tacos");
    await waitForAutosave("saved", 3000);
    check(
      JSON.stringify(puts.slice(before).map((p) => p.selection)) ===
        JSON.stringify([[ballot.options[0].id], [ballot.options[2].id]]),
      "a change while a retry waits sends the newest state, not the stale one",
      puts.slice(before),
    );
    check((await storedLabels(ballot)) === "Tacos", "and stores it");
  }

  // --- Definitive failures are not retried -----------------------------------------
  {
    const ballot = await createBallot({ submission_mode: "editable" });
    await open(`/b/${ballot.token}`);
    const before = puts.length;
    intercept = { status: 422, body: { error: "invalid_input", field: "option_ids[0]", message: "not an Option ID" } };
    await choose("Pizza");
    await waitForAutosave("failed");
    const status = await autosaveStatus();
    check(
      status[1].includes("Not saved.") && status[1].includes("Could not save: not an Option ID."),
      "422: says Not saved, with the reason",
      status,
    );
    await sleep(2500);
    check(puts.length - before === 1, "422: no automatic retry", puts.slice(before));
    check((await cardState("Pizza")) === "failed", "422: the Option stays marked as not saved");
    // Leaving now: the browser asks; the voter stays.
    stayOnBeforeUnload = true;
    const dialogs = jsDialogs.length;
    await send("Page.navigate", { url: `${root}/b/${ballot.token}` }).catch(() => {});
    await waitFor("the beforeunload prompt", () => jsDialogs.length > dialogs, 3000).catch(() => false);
    stayOnBeforeUnload = false;
    check(jsDialogs[dialogs] === "beforeunload", "leaving with a failed save asks for confirmation", jsDialogs.slice(dialogs));
    await sleep(300);
    check(await sameDocument(), "staying keeps the page");
    intercept = null;
    await clickButton("Retry now");
    await waitForAutosave("saved");
    check((await storedLabels(ballot)) === "Pizza", "Retry now saves");
  }

  // --- 409/404 while autosaving: a dialog, then a reload -------------------------------
  const conflicts = [
    { body: { error: "voting_closed" }, status: 409, text: "Voting has closed." },
    { body: { error: "already_answered" }, status: 409, text: "already been answered" },
    { body: { error: "ballot_changed" }, status: 409, text: "changed by its organiser" },
    { body: { error: "not_found" }, status: 404, text: "could not be found" },
  ];
  for (const forced of conflicts) {
    const name = `autosave ${forced.status} ${forced.body.error}`;
    const ballot = await createBallot({ submission_mode: "editable" });
    await open(`/b/${ballot.token}`);
    const before = puts.length;
    intercept = { status: forced.status, body: forced.body };
    await choose("Pizza");
    await waitForDialog("error");
    intercept = null;
    const text = await dialogText();
    check(text.includes(forced.text), `${name}: says "${forced.text}"`, text);
    await sleep(1500);
    check(puts.length - before === 1, `${name}: not retried`);
    const dialogs = jsDialogs.length;
    await clickButton("Reload the page");
    await waitForReload();
    check(jsDialogs.length === dialogs, `${name}: reloads the page without a leave prompt`, jsDialogs.slice(dialogs));
  }

  // --- An accent colour: the in-place refresh keeps to the policy -------------
  // The page's accent colour is an inline <style> allowed by the page's
  // nonce; the fresh page fetched for the refresh carries a new one.
  {
    const ballot = await createBallot({ submission_mode: "editable", accent_color: "#aa3300" });
    await open(`/b/${ballot.token}`);
    const before = problems.length;
    await choose("Pizza");
    await waitFor("the refreshed view", () => evaluate(`document.querySelector(".status")?.dataset.state === "answered"`));
    check(await sameDocument(), "an accent colour: the page updates in place");
    const accent = await evaluate(`getComputedStyle(document.documentElement).getPropertyValue("--accent").trim()`);
    check(accent === "#aa3300", "an accent colour: it still applies after the update", accent);
    await sleep(200);
    check(problems.length === before, "an accent colour: the update causes no CSP violation", problems.slice(before));
  }

  // --- Final: the confirmation dialog -------------------------------------------
  {
    const ballot = await createBallot({ submission_mode: "final", theme: "minimal" });
    await open(`/b/${ballot.token}`);
    check(await visible('button[type="submit"]'), "a Final Ballot keeps its Submit button");
    check((await evaluate(`document.querySelector(".vota-autosave")`)) === null, "a Final Ballot is not autosaved");
    const before = puts.length;
    await choose("Pizza");
    await sleep(600);
    check(puts.length === before, "choosing on a Final Ballot sends nothing yet");
    await click('button[type="submit"]');
    await waitForDialog("confirm");
    const text = await dialogText();
    check(
      text.includes("Pizza") && text.includes("You cannot change your answer after you confirm it."),
      "a Final Ballot asks for confirmation with the no-JS page's warning",
      text,
    );
    check(
      (await evaluate("document.activeElement.textContent")) === "Go back and choose again",
      "the confirmation starts on the safe choice",
    );
    await send("Input.dispatchKeyEvent", { type: "keyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    await send("Input.dispatchKeyEvent", { type: "keyUp", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    await waitFor("dialog closed", async () => (await dialogState()) === null);
    check((await voterView(ballot.token)).status === "unanswered", "Escape cancels: nothing recorded");
    await clickButton("Continue");
    await waitForDialog("confirm");
    await clickButton("Go back and choose again");
    await waitFor("dialog closed", async () => (await dialogState()) === null);
    check((await voterView(ballot.token)).status === "unanswered", "going back records nothing");
    check(await evaluate(`!!document.querySelector("form")`), "going back keeps the form");

    await clickButton("Continue");
    await waitForDialog("confirm");
    await clickButton("Confirm and submit");
    await waitForDialog("saved");
    check((await dialogText()).includes("Saved ✓"), "saving ends in Saved ✓", await dialogText());
    check((await evaluate("document.activeElement.textContent")) === "OK", "the Saved dialog's button has focus");
    check((await voterView(ballot.token)).status === "answered", "confirming records the Response");
    await clickButton("OK");
    await waitFor("the read-only view", () => evaluate(`!document.querySelector("form")`));
    check(await sameDocument(), "the read-only view replaces the form without a reload");
    check(
      (await evaluate(`document.querySelector(".recorded")?.textContent`)) === "Your answer has been recorded.",
      "the page shows the recorded answer",
    );
    check((await evaluate(`document.activeElement.tagName`)) === "H1", "focus moves to the page heading after the update");
  }

  // --- Final: nothing selected ----------------------------------------------------------
  {
    const ballot = await createBallot({ choice_mode: "multiple", submission_mode: "final", theme: "classic" });
    await open(`/b/${ballot.token}`);
    const before = requested.length;
    await click('button[type="submit"]');
    await waitForDialog("error");
    check((await dialogText()).includes("Select at least one Option."), "an empty selection is refused in the page");
    check(!requested.slice(before).some((url) => url.includes("/api/")), "an empty selection sends nothing");
    // Dialog styles follow the Theme's custom properties.
    const colours = await evaluate(`(() => {
      const s = getComputedStyle(document.querySelector("dialog.vota-dialog"));
      return [s.backgroundColor, s.color, s.animationName];
    })()`);
    check(
      colours[0] === (await evaluate(`getComputedStyle(document.querySelector("main")).backgroundColor`)) &&
        colours[1] === (await evaluate(`getComputedStyle(document.body).color`)),
      "the dialog uses the Theme's surface and text colours",
      colours,
    );
    check(colours[2] === "vota-dialog-in", "the dialog animates in by default", colours[2]);
    await clickButton("OK");

    await send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] });
    await click('button[type="submit"]');
    await waitForDialog("error");
    check(
      (await evaluate(`getComputedStyle(document.querySelector("dialog.vota-dialog")).animationName`)) === "none",
      "with prefers-reduced-motion the dialog does not animate",
    );
    await clickButton("OK");
    await send("Emulation.setEmulatedMedia", { features: [] });
  }

  // --- Final: error codes (the voter API's answer forced) ------------------------------
  const cases = [
    { body: { error: "voting_closed" }, status: 409, text: "Voting has closed.", reloads: true },
    { body: { error: "already_answered" }, status: 409, text: "already been answered", reloads: true },
    { body: { error: "ballot_changed" }, status: 409, text: "changed by its organiser", reloads: true },
    {
      body: { error: "invalid_input", field: "option_ids[0]", message: "not an Option ID" },
      status: 422,
      text: "Could not save: not an Option ID.",
      reloads: false,
    },
    { body: { error: "too_many_requests" }, status: 429, text: "Too many requests, try again later.", reloads: false },
    { body: { error: "internal" }, status: 500, text: "Could not save, please try again.", reloads: false },
    { network: true, text: "Could not save, check your connection.", reloads: false },
  ];
  for (const forced of cases) {
    const name = `Final, ${forced.network ? "network error" : `${forced.status} ${forced.body.error}`}`;
    const ballot = await createBallot({ submission_mode: "final" });
    await open(`/b/${ballot.token}`);
    intercept = forced.network ? "network" : { status: forced.status, body: forced.body };
    await choose("Pizza");
    await click('button[type="submit"]');
    await waitForDialog("confirm");
    await clickButton("Confirm and submit");
    await waitForDialog("error");
    intercept = null;
    const text = await dialogText();
    check(text.includes(forced.text), `${name}: says "${forced.text}"`, text);
    await evaluate(`document.querySelector("dialog.vota-dialog button").click()`);
    if (forced.reloads) {
      await waitForReload();
      check(true, `${name}: reloads the page`);
    } else {
      await waitFor("dialog closed", async () => (await dialogState()) === null);
      await sleep(300);
      check(await sameDocument(), `${name}: keeps the page`);
      check(
        await evaluate(`document.querySelector("input[value='${ballot.options[0].id}']").checked`),
        `${name}: keeps the selection`,
      );
    }
  }

  // --- The admin replaced the Options while the page was open ------------------------
  {
    const ballot = await createBallot({ submission_mode: "editable" });
    await open(`/b/${ballot.token}`);
    await replaceOptions(ballot, ["Curry", "Soup"]);
    await choose("Pizza");
    await waitForDialog("error");
    const text = await dialogText();
    check(
      text.includes("This Ballot was changed by its organiser"),
      "a replaced Option: says the Ballot was changed by its organiser",
      text,
    );
    await clickButton("Reload the page");
    await waitForReload();
    const labels = await evaluate(`[...document.querySelectorAll("label")].map(l => l.textContent.trim()).join(",")`);
    check(labels === "Curry,Soup", "a replaced Option: the reload shows the new Options", labels);
    check((await voterView(ballot.token)).status === "unanswered", "a replaced Option: nothing recorded");
  }

  // --- The admin changed the wording while the page was open -------------------------
  for (const submission_mode of ["final", "editable"]) {
    const ballot = await createBallot({ submission_mode, title: "Do you approve proposal X?" });
    await open(`/b/${ballot.token}`);
    await retitle(ballot, "Do you REJECT proposal X?");
    await choose("Pizza");
    if (submission_mode === "final") {
      await click('button[type="submit"]');
      await waitForDialog("confirm");
      await clickButton("Confirm and submit");
    }
    await waitForDialog("error");
    const text = await dialogText();
    check(
      text.includes("This Ballot was changed by its organiser"),
      `${submission_mode}: a changed title since the page loaded is refused as a changed Ballot`,
      text,
    );
    check(
      (await voterView(ballot.token)).status === "unanswered",
      `${submission_mode}: nothing recorded against the old wording`,
    );
    await clickButton("Reload the page");
    await waitForReload();
    const heading = await evaluate(`document.querySelector("h1").textContent`);
    check(heading === "Do you REJECT proposal X?", `${submission_mode}: the reload shows the new wording`, heading);
    // The reloaded page carries the new revision: saving now works.
    await choose("Sushi");
    if (submission_mode === "final") {
      await click('button[type="submit"]');
      await waitForDialog("confirm");
      await clickButton("Confirm and submit");
      await waitForDialog("saved");
      await clickButton("OK");
      await waitFor("dialog closed", async () => (await dialogState()) === null);
    } else {
      await waitForAutosave("saved");
    }
    const saved = await voterView(ballot.token);
    check(
      saved.status === "answered" && saved.response.option_ids[0] === ballot.options[1].id,
      `${submission_mode}: after the reload the answer is saved`,
      saved,
    );
  }

  // --- The countdown reaching zero ---------------------------------------------------
  for (const theme of ["classic", "bare"]) {
    const ballot = await createBallot({
      submission_mode: "editable",
      theme,
      voting_deadline: new Date(Date.now() + 6000).toISOString(),
    });
    await open(`/b/${ballot.token}`);
    if (theme === "classic") {
      const countdown = await evaluate(`document.querySelector(".vota-countdown").textContent`);
      check(/^in \d seconds?$/.test(countdown), "near the deadline the countdown counts seconds", countdown);
      await waitFor("closing", () => evaluate(`document.querySelector(".vota-countdown").textContent === "Voting has closed."`), 10000);
    } else {
      check((await evaluate(`document.querySelector(".vota-countdown")`)) === null, "bare: a template without a placeholder gets no countdown");
      await waitFor("closing", () => evaluate(`[...document.querySelector("form").elements].every(e => e.disabled)`), 10000);
    }
    check(
      await evaluate(`[...document.querySelector("form").elements].every(e => e.disabled)`),
      `${theme}: at the deadline the form is disabled`,
    );
    check(
      (await evaluate(`document.querySelector("[aria-live].vota-visually-hidden").textContent`)) === "Voting has closed.",
      `${theme}: the closing is announced`,
    );
    await waitForReload();
    await waitFor("closed view", () => evaluate(`!!document.querySelector(".closed")`));
    check(
      (await evaluate(`document.querySelector(".closed").textContent`)) === "No answer was given.",
      `${theme}: then the page reloads to the closed view`,
    );
    await evaluate("window.__sameDocument = true");
    await sleep(2500);
    check(await sameDocument(), `${theme}: the closed view stays (no reload loop)`);
  }

  // --- Without JavaScript --------------------------------------------------------------
  {
    await send("Emulation.setScriptExecutionDisabled", { value: true });
    const ballot = await createBallot({ submission_mode: "final" });
    await open(`/b/${ballot.token}`);
    check(
      (await evaluate(`document.querySelector(".vota-autosave, dialog")`)) === null &&
        (await evaluate(`document.querySelector(".vota-countdown").textContent`)) === "",
      "without JavaScript the page is the plain form",
    );
    await choose("Sushi");
    let loaded = waitForLoad();
    await clickButton("Continue");
    await loaded;
    check(
      (await evaluate(`document.querySelector("h2")?.textContent`)) === "Confirm your answer",
      "without JavaScript the form posts to the confirmation page",
    );
    loaded = waitForLoad();
    await clickButton("Confirm and submit");
    await loaded;
    check(
      (await evaluate(`location.pathname`)) === `${PREFIX}/b/${ballot.token}` &&
        (await evaluate(`document.querySelector(".recorded")?.textContent`)) === "Your answer has been recorded.",
      "confirming redirects to the read-only page",
    );
    check((await voterView(ballot.token)).status === "answered", "without JavaScript the Response is recorded");

    // Editable: the Submit button is there and posts.
    const editable = await createBallot({ submission_mode: "editable" });
    await open(`/b/${editable.token}`);
    check(await visible('button[type="submit"]'), "without JavaScript an Editable Ballot shows its Submit button");
    await choose("Tacos");
    loaded = waitForLoad();
    await clickButton("Save answer");
    await loaded;
    check(
      (await evaluate(`location.pathname`)) === `${PREFIX}/b/${editable.token}` &&
        (await evaluate(`document.querySelector(".status").dataset.state`)) === "answered",
      "without JavaScript saving redirects to the page, now answered",
    );
    check((await storedLabels(editable)) === "Tacos", "without JavaScript the Editable Response is recorded");

    // The admin replaced the Options while the plain form was open.
    const changed = await createBallot({ submission_mode: "final" });
    await open(`/b/${changed.token}`);
    await replaceOptions(changed, ["Curry", "Soup"]);
    await choose("Sushi");
    loaded = waitForLoad();
    await clickButton("Continue");
    await loaded;
    const notice = await evaluate(`document.querySelector(".message")?.textContent ?? ""`);
    check(
      notice.startsWith("This Ballot was changed by its organiser"),
      "without JavaScript a replaced Option shows the changed Ballot with the notice",
      notice,
    );
    const fresh = await evaluate(`[...document.querySelectorAll("label")].map(l => l.textContent.trim()).join(",")`);
    check(fresh === "Curry,Soup", "without JavaScript the page then offers the new Options", fresh);
    check((await voterView(changed.token)).status === "unanswered", "without JavaScript nothing was recorded");

    // The admin negated the title between the confirmation step and confirming.
    const negated = await createBallot({ submission_mode: "final", title: "Do you approve proposal X?" });
    await open(`/b/${negated.token}`);
    await choose("Pizza");
    loaded = waitForLoad();
    await clickButton("Continue");
    await loaded;
    check(
      (await evaluate(`document.querySelector("h1").textContent`)) === "Do you approve proposal X?",
      "without JavaScript the confirmation shows the wording the voter saw",
    );
    await retitle(negated, "Do you REJECT proposal X?");
    loaded = waitForLoad();
    await clickButton("Confirm and submit");
    await loaded;
    const refused = await evaluate(`document.querySelector(".message")?.textContent ?? ""`);
    check(
      refused.startsWith("This Ballot was changed by its organiser"),
      "without JavaScript a confirmation of changed wording is refused with the notice",
      refused,
    );
    check(
      (await evaluate(`document.querySelector("h1").textContent`)) === "Do you REJECT proposal X?",
      "without JavaScript the page then shows the new wording",
    );
    check((await voterView(negated.token)).status === "unanswered", "without JavaScript nothing was recorded for it");
    await send("Emulation.setScriptExecutionDisabled", { value: false });
  }

  // --- The modules ---------------------------------------------------------------------
  const modules = ["main", "config", "dom", "time", "api", "dialog", "autosave", "refresh"];
  const version = requested
    .map((url) => url.slice(root.length).match(/^\/assets\/app\/([0-9a-f]{16})\/main\.js$/))
    .find(Boolean)?.[1];
  check(!!version, "the entry module loads from the content-versioned directory on our own host");
  check(
    modules.every((name) => requested.includes(`${root}/assets/app/${version}/${name}.js`)),
    "every module loads from the same versioned directory (relative imports)",
    requested.filter((url) => url.includes("/assets/app/")),
  );
  check(
    requested.some((url) => /^\/assets\/app\/ballot\.css\?v=[0-9a-f]{16}$/.test(url.slice(root.length)) && url.startsWith(root)),
    "the stylesheet loads from our own host, by content version",
  );
  const cacheControl = (await fetch(`${root}/assets/app/${version}/autosave.js`)).headers.get("cache-control");
  check(cacheControl === "public, max-age=31536000, immutable", "the versioned modules are cached for good", cacheControl);
  check(problems.length === 0, "no console errors or CSP violations", problems);
  // The harness itself notices a CSP violation (an inline script).
  await evaluate(`document.head.appendChild(document.createElement("script")).textContent = "window.__inline = 1"`);
  await waitFor("the CSP report", async () => problems.length > 0, 3000).catch(() => false);
  check(
    problems.length > 0 && (await evaluate("window.__inline")) === undefined,
    "the policy blocks inline script, and this check would notice",
    problems,
  );
  check(
    requested.every((url) => url.startsWith(origin) || url.startsWith("data:") || url === "about:blank"),
    "the page requests nothing from other hosts",
    requested.filter((url) => !url.startsWith(origin)),
  );
  // Chromium probes the host root for `/favicon.ico` by itself; outside the
  // prefix it gets a plain 404, which the rate limit ignores.
  const fromThePage = requested.filter((url) => url !== `${origin}/favicon.ico`);
  check(
    fromThePage.every((url) => url.startsWith(`${root}/`) || url.startsWith("data:") || url === "about:blank"),
    `the page requests nothing outside the base path ${BASE_PATH}`,
    fromThePage.filter((url) => !url.startsWith(`${root}/`)),
  );
} catch (error) {
  failures++;
  console.log(`FAIL ${error.stack}`);
  console.log("page problems:", problems);
  console.log("dialog:", await dialogText().catch(() => "?"));
} finally {
  socket.close();
  browser.kill();
  server.kill();
  await sleep(200);
  rmSync(dir, { recursive: true, force: true });
}

console.log(failures === 0 ? "\nall browser checks passed" : `\n${failures} browser check(s) failed`);
process.exit(failures === 0 ? 0 : 1);
