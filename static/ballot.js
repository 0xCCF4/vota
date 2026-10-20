// Vota: the voting page's enhancement. Plain JavaScript, no build step,
// no third-party code, no requests to any other host.
//
// The app includes this script on every voting page (see
// src/web/enhancement.rs), with what it needs in data-* attributes on its
// own <script> tag. Without it (or without JavaScript) the page works as a
// plain form (post/redirect/get). With it:
// - the Voting Deadline is shown in the voter's local time zone, with a
//   live countdown; when it reaches zero the form is disabled and the page
//   reloads to show the closed view;
// - the form is saved through the voter API (PUT, JSON) without leaving the
//   page, with a dialog showing "Saving…", then "Saved ✓" or what went
//   wrong; a Final Ballot first asks for confirmation in a dialog. It sends
//   the Ballot's revision as rendered, so an admin edit since the page was
//   loaded refuses the Response ("This Ballot was changed") instead of
//   recording it against wording the voter never saw.
//
// It relies on nothing in the page but the no-JS form (a method="post" form
// with "option" inputs and their <label for>s) and, for the local time,
// <time datetime> elements, so it works with any Theme's templates.
(function () {
  "use strict";

  var script = document.currentScript;
  if (!script || !script.dataset || !script.dataset.page) {
    return;
  }
  var SCRIPT_SRC = script.getAttribute("src");

  var MINUTE = 60 * 1000;
  var HOUR = 60 * MINUTE;
  var DAY = 24 * HOUR;
  // How long a save may take before it counts as a connection problem.
  var SAVE_TIMEOUT = 20 * 1000;
  // How long "Voting has closed" shows before the page reloads.
  var CLOSE_RELOAD_DELAY = 1500;

  var config = readConfig(script);
  var countdownTimer = null;
  var busy = false;
  var reloadWhenIdle = false;
  var canDialog =
    typeof HTMLDialogElement === "function" &&
    typeof document.createElement("dialog").showModal === "function" &&
    typeof window.fetch === "function";

  // Announcements for screen readers outside the dialog (e.g. "Voting has
  // closed"): a polite live region, present from the start so changes to
  // it are read out.
  var live = element("p", "vota-visually-hidden");
  live.setAttribute("aria-live", "polite");
  live.setAttribute("aria-atomic", "true");

  var dialog = canDialog ? createDialog() : null;

  /** What the server told us, from a <script> tag of this script. */
  function readConfig(tag) {
    var data = tag.dataset;
    var deadline = Date.parse(data.deadline);
    var serverNow = Date.parse(data.now);
    return {
      api: data.api,
      page: data.page,
      deadline: deadline,
      // Server "now" minus client "now": corrects a skewed client clock.
      offset: isNaN(serverNow) ? 0 : serverNow - Date.now(),
      open: data.open === "true",
      // The Ballot's revision as rendered: the Response is recorded only if
      // the Ballot still stands as the voter saw it.
      revision: Number(data.revision),
      multiple: data.choiceMode === "multiple",
      needsConfirmation: data.needsConfirmation === "true",
    };
  }

  function now() {
    return Date.now() + config.offset;
  }

  function element(tag, className, text) {
    var node = document.createElement(tag);
    if (className) {
      node.className = className;
    }
    if (text !== undefined) {
      node.textContent = text;
    }
    return node;
  }

  // --- Local time and countdown -------------------------------------------

  var dateFormat = (function () {
    if (typeof Intl !== "object" || !Intl.DateTimeFormat) {
      return null;
    }
    // The page is in English: use the voter's English variant if they have
    // one, otherwise British English (as the server's own format reads).
    var language = navigator.language || "";
    var locale = /^en(-|$)/i.test(language) ? language : "en-GB";
    try {
      return new Intl.DateTimeFormat(locale, {
        weekday: "long",
        day: "numeric",
        month: "long",
        year: "numeric",
        hour: "2-digit",
        minute: "2-digit",
        timeZoneName: "short",
      });
    } catch (error) {
      return null;
    }
  })();

  /** Every <time datetime> in the voter's local time zone. */
  function localizeTimes() {
    if (!dateFormat) {
      return;
    }
    var times = document.querySelectorAll("time[datetime]");
    for (var i = 0; i < times.length; i++) {
      var time = times[i];
      var instant = Date.parse(time.getAttribute("datetime"));
      if (isNaN(instant) || time.hasAttribute("data-vota-local")) {
        continue;
      }
      // The server's rendering (in its configured zone) stays as a tooltip.
      time.title = time.textContent;
      time.textContent = dateFormat.format(new Date(instant));
      time.setAttribute("data-vota-local", "");
    }
  }

  function plural(count, unit) {
    return count + " " + unit + (count === 1 ? "" : "s");
  }

  /** "6 days, 23 hours", "2 hours, 5 minutes", "4 minutes, 9 seconds", … */
  function remainingText(ms) {
    var seconds = Math.floor(ms / 1000);
    var days = Math.floor(seconds / 86400);
    var hours = Math.floor((seconds % 86400) / 3600);
    var minutes = Math.floor((seconds % 3600) / 60);
    var secs = seconds % 60;
    if (days > 0) {
      return plural(days, "day") + ", " + plural(hours, "hour");
    }
    if (hours > 0) {
      return plural(hours, "hour") + ", " + plural(minutes, "minute");
    }
    if (minutes > 0) {
      return plural(minutes, "minute") + ", " + plural(secs, "second");
    }
    return plural(secs, "second");
  }

  /** Where the countdown goes: beside the deadline, or before the form. */
  function countdownElement() {
    var existing = document.querySelector(".vota-countdown");
    if (existing) {
      return existing;
    }
    var countdown = element("span", "vota-countdown");
    // A timer: read on demand, never announced on every tick.
    countdown.setAttribute("role", "timer");
    var time = deadlineTime();
    if (time && time.parentNode) {
      time.parentNode.appendChild(document.createTextNode(" "));
      time.parentNode.appendChild(countdown);
      return countdown;
    }
    var holder = element("p", "vota-countdown-line");
    holder.appendChild(countdown);
    var form = votingForm();
    var main = document.querySelector("main") || document.body;
    if (form && form.parentNode) {
      form.parentNode.insertBefore(holder, form);
    } else {
      main.insertBefore(holder, main.firstChild);
    }
    return countdown;
  }

  /** The <time> showing the Voting Deadline, if the page has one. */
  function deadlineTime() {
    var times = document.querySelectorAll("time[datetime]");
    for (var i = 0; i < times.length; i++) {
      if (Date.parse(times[i].getAttribute("datetime")) === config.deadline) {
        return times[i];
      }
    }
    return null;
  }

  function startCountdown() {
    stopCountdown();
    if (!config.open || isNaN(config.deadline)) {
      return;
    }
    var countdown = countdownElement();
    var tick = function () {
      var remaining = config.deadline - now();
      if (remaining <= 0) {
        countdown.textContent = "Voting has closed.";
        votingClosed();
        return;
      }
      var text = "(" + remainingText(remaining) + " left)";
      if (countdown.textContent !== text) {
        countdown.textContent = text;
      }
      // Wake up just after the shown value changes: every second in the
      // last hour, every minute before.
      var step = remaining > HOUR ? MINUTE : 1000;
      countdownTimer = window.setTimeout(tick, (remaining % step) + 20);
    };
    tick();
  }

  function stopCountdown() {
    if (countdownTimer !== null) {
      window.clearTimeout(countdownTimer);
      countdownTimer = null;
    }
  }

  /** The deadline has passed: lock the form, say so, then reload. */
  function votingClosed() {
    stopCountdown();
    config.open = false;
    var form = votingForm();
    if (form) {
      for (var i = 0; i < form.elements.length; i++) {
        form.elements[i].disabled = true;
      }
    }
    live.textContent = "Voting has closed.";
    if (dialog && dialog.open && dialog.getAttribute("data-state") === "confirm") {
      settle("cancel");
    }
    if (busy || (dialog && dialog.open)) {
      // A save is under way or a result is showing: reload once it's done.
      reloadWhenIdle = true;
    } else {
      window.setTimeout(reload, CLOSE_RELOAD_DELAY);
    }
  }

  /** The server-rendered page, freshly (never re-posting a form). */
  function reload() {
    window.location.replace(config.page);
  }

  // --- The voting form ----------------------------------------------------

  /** The no-JS voting form: posts checkable "option" inputs. */
  function votingForm() {
    var forms = document.querySelectorAll("form");
    for (var i = 0; i < forms.length; i++) {
      if (isVotingForm(forms[i])) {
        return forms[i];
      }
    }
    return null;
  }

  function isVotingForm(form) {
    if ((form.getAttribute("method") || "").toLowerCase() !== "post") {
      return false;
    }
    var inputs = form.querySelectorAll('input[name="option"]');
    if (inputs.length === 0) {
      return false;
    }
    for (var i = 0; i < inputs.length; i++) {
      if (inputs[i].type !== "radio" && inputs[i].type !== "checkbox") {
        return false;
      }
    }
    return true;
  }

  function onSubmit(event) {
    var form = event.target;
    if (!dialog || !config.open || !isVotingForm(form)) {
      return;
    }
    event.preventDefault();
    if (busy) {
      return;
    }
    var inputs = form.querySelectorAll('input[name="option"]');
    var chosen = [];
    for (var i = 0; i < inputs.length; i++) {
      if (inputs[i].checked) {
        chosen.push(inputs[i]);
      }
    }
    if (chosen.length === 0) {
      showResult(
        "error",
        "Nothing selected",
        config.multiple ? "Select at least one Option." : "Select exactly one Option.",
        null
      );
      return;
    }
    var ids = chosen.map(function (input) {
      return input.value;
    });
    if (config.needsConfirmation) {
      confirmChoice(chosen.map(labelOf), function () {
        save(ids);
      });
    } else {
      save(ids);
    }
  }

  function labelOf(input) {
    var labels = input.labels;
    if (labels && labels.length) {
      return labels[0].textContent.trim();
    }
    return input.value;
  }

  // --- The dialog ---------------------------------------------------------

  // The open dialog's pending outcome: what to do when the voter answers it
  // (a button, or Escape). Settled at most once, synchronously, so a late
  // "close" event of one dialog can never act on the next one.
  var session = null;

  function createDialog() {
    var node = element("dialog", "vota-dialog");
    node.setAttribute("aria-labelledby", "vota-dialog-title");
    node.setAttribute("aria-describedby", "vota-dialog-message");
    // Focusable itself, for while it has no buttons ("Saving…").
    node.setAttribute("tabindex", "-1");
    // Escape: the dialog's "cancel" answer; while saving, nothing.
    node.addEventListener("cancel", function (event) {
      event.preventDefault();
      settle("cancel");
    });
    // Closed by the browser despite that: also "cancel" (but never act on a
    // dialog that has been reopened since).
    node.addEventListener("close", function () {
      if (!node.open) {
        settle("cancel");
      }
    });
    return node;
  }

  function settle(result) {
    var pending = session;
    session = null;
    if (pending) {
      pending(result);
    }
  }

  function closeDialog() {
    session = null;
    if (dialog.open) {
      dialog.close();
    }
  }

  /** Fills the dialog (opening it if needed): a title, a message, extra
   * content, buttons, and what to do once the voter answers it. */
  function fillDialog(state, title, message, extra, buttons, onAnswer) {
    session = onAnswer || null;
    dialog.setAttribute("data-state", state);
    dialog.replaceChildren();
    var heading = element("h2", "vota-dialog-title");
    heading.id = "vota-dialog-title";
    if (title.node) {
      heading.appendChild(title.node);
    } else {
      heading.textContent = title;
    }
    dialog.appendChild(heading);
    var text = element("p", "vota-dialog-message", message);
    text.id = "vota-dialog-message";
    text.setAttribute("role", "status");
    text.setAttribute("aria-live", "polite");
    dialog.appendChild(text);
    (extra || []).forEach(function (node) {
      dialog.appendChild(node);
    });
    if (buttons.length) {
      var actions = element("div", "vota-dialog-actions");
      buttons.forEach(function (button) {
        actions.appendChild(button);
      });
      dialog.appendChild(actions);
    }
    if (!dialog.isConnected) {
      document.body.appendChild(dialog);
    }
    if (!dialog.open) {
      dialog.showModal();
    }
  }

  /** A dialog button answering the dialog with `result`. */
  function button(text, className, result) {
    var node = element("button", className, text);
    node.type = "button";
    node.addEventListener("click", function () {
      settle(result);
    });
    return node;
  }

  /** The Final confirmation step, as on the no-JS confirmation page. */
  function confirmChoice(labels, onConfirm) {
    var list = element("ul", "vota-selection");
    labels.forEach(function (label) {
      list.appendChild(element("li", null, label));
    });
    var warning = element("p", "vota-warning");
    warning.appendChild(
      element("strong", null, "You cannot change your answer after you confirm it.")
    );
    var confirm = button("Confirm and submit", "vota-primary", "confirm");
    var back = button("Go back and choose again", "vota-secondary", "cancel");
    fillDialog("confirm", "Confirm your answer", "You chose:", [list, warning], [confirm, back], function (result) {
      if (result === "confirm" && config.open) {
        // The dialog stays open and turns into "Saving…".
        onConfirm();
      } else {
        closeDialog();
        focusForm();
      }
    });
    // Start on the safe choice, so a stray Enter doesn't lock the answer in.
    back.focus();
  }

  /** A result in the dialog, with one button; `after` runs once it closes. */
  function showResult(state, title, message, after) {
    var ok = button(after === reload ? "Reload the page" : "OK", "vota-primary", "ok");
    fillDialog(state, title, message, [], [ok], function () {
      closeDialog();
      if (reloadWhenIdle) {
        reload();
      } else if (after) {
        after();
      } else {
        focusForm();
      }
    });
    ok.focus();
  }

  function focusForm() {
    var form = votingForm();
    var target = form && form.querySelector('button[type="submit"], button:not([type])');
    if (target) {
      target.focus();
    }
  }

  // --- Saving -------------------------------------------------------------

  function save(ids) {
    busy = true;
    fillDialog("saving", "Saving your answer", "Saving…", [], []);
    dialog.focus();
    var controller = typeof AbortController === "function" ? new AbortController() : null;
    var timer = window.setTimeout(function () {
      if (controller) {
        controller.abort();
      }
    }, SAVE_TIMEOUT);
    window
      .fetch(config.api, {
        method: "PUT",
        headers: { "Content-Type": "application/json", Accept: "application/json" },
        body: JSON.stringify({ option_ids: ids, revision: config.revision }),
        credentials: "same-origin",
        cache: "no-store",
        signal: controller ? controller.signal : undefined,
      })
      .then(function (response) {
        return response
          .json()
          .catch(function () {
            return {};
          })
          .then(function (body) {
            window.clearTimeout(timer);
            busy = false;
            outcome(response.status, body || {});
          });
      })
      .catch(function () {
        window.clearTimeout(timer);
        busy = false;
        showResult("error", "Not saved", "Could not save, check your connection.", null);
      });
  }

  /** What the voter API's answer means for the voter. */
  function outcome(status, body) {
    if (status >= 200 && status < 300) {
      var check = document.createDocumentFragment();
      check.appendChild(document.createTextNode("Saved "));
      var mark = element("span", null, "✓");
      mark.setAttribute("aria-hidden", "true");
      check.appendChild(mark);
      var editable = body.submission_mode === "editable";
      showResult(
        "saved",
        { node: check },
        editable
          ? "Your answer has been saved. You can change it until voting closes."
          : "Your answer has been recorded.",
        refreshView
      );
      return;
    }
    var error = body.error;
    if (status === 409 && error === "voting_closed") {
      showResult("error", "Voting has closed", "Voting has closed. Your answer could not be accepted.", reload);
    } else if (status === 409 && error === "already_answered") {
      showResult(
        "error",
        "Already answered",
        "This Ballot has already been answered. Your answer cannot be changed.",
        reload
      );
    } else if (status === 409 && error === "ballot_changed") {
      showResult(
        "error",
        "This Ballot was changed",
        "This Ballot was changed by its organiser, so your answer was not saved. Please check it and answer again.",
        reload
      );
    } else if (status === 422) {
      showResult(
        "error",
        "Not saved",
        "Could not save: " + (typeof body.message === "string" ? body.message : "the answer is not valid") + ".",
        null
      );
    } else if (status === 429) {
      showResult("error", "Not saved", "Too many requests, try again later.", null);
    } else if (status === 404) {
      showResult("error", "Not found", "This Ballot could not be found. It may have been deleted.", reload);
    } else {
      showResult("error", "Not saved", "Could not save, please try again.", null);
    }
  }

  // --- Refreshing the page in place ---------------------------------------

  /** The server's view of the page, swapped in without a reload (the
   * stylesheets must match; otherwise, and on any problem, a reload). */
  function refreshView() {
    window
      .fetch(config.page, {
        headers: { Accept: "text/html" },
        credentials: "same-origin",
        cache: "no-store",
      })
      .then(function (response) {
        if (!response.ok) {
          throw new Error("status " + response.status);
        }
        return response.text();
      })
      .then(function (html) {
        var fresh = new DOMParser().parseFromString(withOwnNonce(html), "text/html");
        var tag = fresh.querySelector('script[src="' + SCRIPT_SRC + '"]');
        if (!tag || styles(fresh) !== styles(document) || !fresh.body) {
          throw new Error("the page changed");
        }
        config = readConfig(tag);
        document.title = fresh.title;
        var nodes = Array.prototype.slice.call(fresh.body.childNodes);
        document.body.replaceChildren.apply(
          document.body,
          nodes.map(function (node) {
            return document.adoptNode(node);
          })
        );
        document.body.appendChild(live);
        enhance();
        var heading =
          document.querySelector("main h1") || document.querySelector("h1") || document.querySelector("main");
        if (heading) {
          heading.setAttribute("tabindex", "-1");
          heading.focus();
        }
      })
      .catch(reload);
  }

  /** The fresh page's markup with the nonce of its inline styles (each
   * response has its own) replaced by this page's: the parsed copy falls
   * under this page's Content Security Policy, which would refuse the new
   * nonce and report a violation. A fresh page with nonce'd styles where
   * this one has none has changed its styles: a reload. (Matches the
   * double-quoted `nonce="…"` of the built-in templates.) */
  function withOwnNonce(html) {
    var own = document.querySelector("style[nonce]");
    var nonce = own ? own.nonce : "";
    if (!nonce) {
      if (/<style\b[^>]*\snonce="/i.test(html)) {
        throw new Error("the page changed");
      }
      return html;
    }
    return html.replace(/(<style\b[^>]*\snonce=)"[^"]*"/gi, function (_, start) {
      return start + '"' + nonce + '"';
    });
  }

  /** The page's stylesheets and inline styles, to compare two pages. */
  function styles(doc) {
    var parts = [];
    var nodes = doc.querySelectorAll('link[rel="stylesheet"], style');
    for (var i = 0; i < nodes.length; i++) {
      parts.push(nodes[i].getAttribute("href") || nodes[i].textContent);
    }
    return parts.join("\n");
  }

  // --- Start --------------------------------------------------------------

  function enhance() {
    localizeTimes();
    startCountdown();
  }

  function start() {
    document.body.appendChild(live);
    enhance();
    if (dialog) {
      document.addEventListener("submit", onSubmit);
    }
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start);
  } else {
    start();
  }
})();
