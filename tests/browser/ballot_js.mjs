// Browser check of the voting page's script (static/ballot.js) in headless
// Chromium, driven over the DevTools protocol. Dependency-free: Node (with
// its built-in fetch and WebSocket) and Chromium only. Not part of
// `cargo test`; run it by hand after changing the script:
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
// It starts the debug binary (`target/debug/vota`, or $VOTA_BIN) on a
// free local port against a temp database, creates Ballots through the admin
// API, and checks with JavaScript enabled: the local-time deadline and
// countdown, saving through the voter API with the saving/saved dialog, the
// Final confirmation dialog, the error dialogs per error code (forced by
// intercepting the API request), an Option the admin replaced or a title
// the admin changed while the page was open (409 `ballot_changed`, then a
// reload; the script sends the page's revision), the countdown
// closing the page, focus and reduced motion, an accent-colour Ballot's
// in-place update (its nonce'd style), the content-versioned script
// URL, no requests to other hosts and no CSP violations; and, with
// JavaScript disabled, the plain form flow, the replaced-Option notice and a
// title changed between the confirmation step and confirming.

import { spawn, execFileSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
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
// Voter API calls to answer with a canned response (or fail) instead.
let intercept = null;
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
  if (method === "Fetch.requestPaused") {
    const canned = intercept;
    if (!canned) {
      await send("Fetch.continueRequest", { requestId: params.requestId });
    } else if (canned === "network") {
      await send("Fetch.failRequest", { requestId: params.requestId, errorReason: "InternetDisconnected" });
    } else {
      await send("Fetch.fulfillRequest", {
        requestId: params.requestId,
        responseCode: canned.status,
        responseHeaders: [{ name: "content-type", value: "application/json" }],
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
  const result = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
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
const waitForReload = () => waitFor("a reload", async () => (await evaluate("window.__sameDocument")) !== true);

try {
  // --- Editable: local time, countdown, save in place ------------------------
  {
    const ballot = await createBallot({ submission_mode: "editable", theme: "dark" });
    await open(`/b/${ballot.token}`);
    const time = await evaluate(`document.querySelector("time").textContent`);
    check(/(EST|EDT|GMT-[45])/.test(time), "the deadline shows in the voter's time zone", time);
    const title = await evaluate(`document.querySelector("time").title`);
    check(/CET|CEST/.test(title), "the server's rendering stays as a tooltip", title);
    const countdown = await evaluate(`document.querySelector(".vota-countdown").textContent`);
    check(/^\((2 days, 23 hours|3 days, 0 hours) left\)$/.test(countdown), "the countdown shows the remaining time", countdown);

    await choose("Sushi");
    await click('button[type="submit"]');
    await waitForDialog("saved");
    check((await dialogText()).includes("Saved ✓"), "saving ends in Saved ✓", await dialogText());
    check(
      (await evaluate("document.activeElement.textContent")) === "OK",
      "the Saved dialog's button has focus",
    );
    const saved = await voterView(ballot.token);
    check(
      saved.status === "answered" && saved.response.option_ids[0] === ballot.options[1].id,
      "the Response was recorded through the voter API",
      saved.response,
    );
    await clickButton("OK");
    await waitFor("the refreshed view", () => evaluate(`!!document.querySelector(".recorded")`));
    check(await sameDocument(), "the page updates in place, without a reload");
    check(
      await evaluate(`document.querySelector("input[value='${ballot.options[1].id}']").checked`),
      "the form stays, pre-filled with the current answer",
    );
    check(
      (await evaluate(`document.querySelector("button[type=submit]").textContent`)) === "Change answer",
      "the form offers to change the answer",
    );
    check(!!(await evaluate(`document.querySelector(".vota-countdown")`)), "the countdown is back after the update");
    check(
      (await evaluate(`document.activeElement.tagName`)) === "H1",
      "focus moves to the page heading after the update",
    );

    // Changing the answer works again in the same document.
    await choose("Tacos");
    await click('button[type="submit"]');
    await waitForDialog("saved");
    check(
      (await voterView(ballot.token)).response.option_ids[0] === ballot.options[2].id,
      "a second change is saved too",
    );
    await clickButton("OK");
    await waitFor("dialog closed", async () => (await dialogState()) === null);
  }

  // --- An accent colour: the in-place refresh keeps to the policy -------------
  // The page's accent colour is an inline <style> allowed by the page's
  // nonce; the fresh page fetched for the refresh carries a new one.
  {
    const ballot = await createBallot({ submission_mode: "editable", accent_color: "#aa3300" });
    await open(`/b/${ballot.token}`);
    const before = problems.length;
    await choose("Pizza");
    await click('button[type="submit"]');
    await waitForDialog("saved");
    await clickButton("OK");
    await waitFor("the refreshed view", () => evaluate(`!!document.querySelector(".recorded")`));
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
    await choose("Pizza");
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
    check((await voterView(ballot.token)).status === "answered", "confirming records the Response");
    await clickButton("OK");
    await waitFor("the read-only view", () => evaluate(`!document.querySelector("form")`));
    check(await sameDocument(), "the read-only view replaces the form without a reload");
    check(
      (await evaluate(`document.querySelector(".recorded")?.textContent`)) === "Your answer has been recorded.",
      "the page shows the recorded answer",
    );
  }

  // --- Nothing selected ----------------------------------------------------------
  {
    const ballot = await createBallot({ choice_mode: "multiple", submission_mode: "editable", theme: "classic" });
    await open(`/b/${ballot.token}`);
    const before = requested.length;
    await click('button[type="submit"]');
    await waitForDialog("error");
    check((await dialogText()).includes("Select at least one Option."), "an empty selection is refused in the page");
    check(!requested.slice(before).some((url) => url.includes("/api/")), "an empty selection sends nothing");
    // Dialog styles follow the Theme's custom properties.
    const colours = await evaluate(`(() => {
      const s = getComputedStyle(document.querySelector("dialog.vota-dialog"));
      return [s.backgroundColor, s.color, s.borderTopColor, s.animationName];
    })()`);
    check(
      colours[0] === "rgb(255, 255, 255)" && colours[1] === "rgb(28, 36, 48)",
      "the dialog uses the Theme's surface and text colours",
      colours,
    );
    check(colours[3] === "vota-dialog-in", "the dialog animates in by default", colours[3]);
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

  // --- Error codes (the voter API's answer forced) -----------------------------------
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
    const name = forced.network ? "network error" : `${forced.status} ${forced.body.error}`;
    const ballot = await createBallot({ submission_mode: "editable" });
    await open(`/b/${ballot.token}`);
    intercept = forced.network ? "network" : { status: forced.status, body: forced.body };
    await choose("Pizza");
    await click('button[type="submit"]');
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
    await click('button[type="submit"]');
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
    await click('button[type="submit"]');
    if (submission_mode === "final") {
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
    await click('button[type="submit"]');
    if (submission_mode === "final") {
      await waitForDialog("confirm");
      await clickButton("Confirm and submit");
    }
    await waitForDialog("saved");
    const saved = await voterView(ballot.token);
    check(
      saved.status === "answered" && saved.response.option_ids[0] === ballot.options[1].id,
      `${submission_mode}: after the reload the answer is saved`,
      saved,
    );
    await clickButton("OK");
    await waitFor("dialog closed", async () => (await dialogState()) === null);
  }

  // --- The countdown reaching zero ---------------------------------------------------
  {
    const ballot = await createBallot({
      submission_mode: "editable",
      voting_deadline: new Date(Date.now() + 6000).toISOString(),
    });
    await open(`/b/${ballot.token}`);
    const countdown = await evaluate(`document.querySelector(".vota-countdown").textContent`);
    check(/^\(\d seconds? left\)$/.test(countdown), "near the deadline the countdown counts seconds", countdown);
    await waitFor("closing", () => evaluate(`document.querySelector(".vota-countdown").textContent === "Voting has closed."`), 10000);
    check(
      await evaluate(`[...document.querySelector("form").elements].every(e => e.disabled)`),
      "at the deadline the form is disabled",
    );
    check(
      (await evaluate(`document.querySelector("[aria-live].vota-visually-hidden").textContent`)) === "Voting has closed.",
      "the closing is announced",
    );
    await waitForReload();
    await waitFor("closed view", () => evaluate(`!!document.querySelector(".closed")`));
    check(
      (await evaluate(`document.querySelector(".closed").textContent`)) === "Voting closed. No answer was given.",
      "then the page reloads to the closed view",
    );
    await evaluate("window.__sameDocument = true");
    await sleep(2500);
    check(await sameDocument(), "the closed view stays (no reload loop)");
  }

  // --- Without JavaScript --------------------------------------------------------------
  {
    await send("Emulation.setScriptExecutionDisabled", { value: true });
    const ballot = await createBallot({ submission_mode: "final" });
    await open(`/b/${ballot.token}`);
    check(
      (await evaluate(`document.querySelector(".vota-countdown, dialog")`)) === null,
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

  check(
    requested.some((url) => /^\/assets\/app\/ballot\.js\?v=[0-9a-f]{16}$/.test(url.slice(root.length)) && url.startsWith(root)) &&
      requested.some((url) => /^\/assets\/app\/ballot\.css\?v=[0-9a-f]{16}$/.test(url.slice(root.length)) && url.startsWith(root)),
    "the script and its stylesheet load from our own host, by content version",
  );
  const script = requested.find((url) => url.startsWith(`${root}/assets/app/ballot.js?v=`));
  const cacheControl = script && (await fetch(script)).headers.get("cache-control");
  check(cacheControl === "public, max-age=31536000, immutable", "the versioned script is cached for good", cacheControl);
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
