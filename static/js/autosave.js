// Vota: autosave of an Editable Ballot's voting form. Every change is saved
// at once through the voter API, so the form needs no Submit button (it
// stays in the page, hidden, for the no-JS flow).
//
// - Strictly serialised: at most one request in flight. Changes made
//   meanwhile update the page at once and are coalesced: the next request,
//   sent once the previous one has finished, carries the latest selection.
// - Multiple choice waits a moment (DEBOUNCE_MULTIPLE) for further clicks;
//   single choice saves at once.
// - Feedback per change: the changed Options' cards get
//   data-vota-save="pending" ("Saving…"), then "saved" (a fading
//   highlight), or "failed" (until resolved); a status line (role="status")
//   says the same in words. A Option's card never claims a state that
//   isn't stored: a checked box that isn't stored is "pending" or "failed".
// - Transient failures (no connection, timeout, 5xx, 429) are retried with
//   backoff (1 s, 2 s, 4 s, 8 s, then every 15 s, for about 2 minutes;
//   at least 429's Retry-After). "Retry now" is always offered, and coming
//   back online or to the tab retries at once. Definitive answers (409,
//   404, other 4xx) are never retried: they go to `onDefinitive`.
// - The rules for how many Options may be chosen are the server's; the
//   page only mirrors them: an empty multiple-choice selection is not sent,
//   unless the Ballot allows no Option. Then unticking everything saves the
//   answer "no Option", and while the Ballot has no Response yet the Save
//   button stays, so the voter can save "no Option" without ticking first.
// - A Response recorded from the admin's Preselection is stored like any
//   other, but not yet the voter's: Save stays, and saving the selection
//   as it stands (even unchanged) makes it theirs.

import { putResponse } from "./api.js";
import { checkedIds, element, optionBox, optionInputs, submitButtons } from "./dom.js";

export const DEBOUNCE_MULTIPLE = 400;
const BACKOFF = [1000, 2000, 4000, 8000];
const BACKOFF_STEADY = 15 * 1000;
const RETRY_FOR = 2 * 60 * 1000;
/** How long a saved Option stays highlighted. */
const SAVED_FOR = 2000;

const OPTION_TEXT = { pending: "Saving…", saved: "Saved", failed: "Not saved" };
const ICON = { idle: "", pending: "", saved: "✓", failed: "!", invalid: "!" };

/** Whether two selections are the same; `null` (no Response stored) is
 * only the same as `null`. */
function same(a, b) {
  if (a === null || b === null) {
    return a === b;
  }
  return a.length === b.length && a.every((id) => b.includes(id));
}

/**
 * Starts autosaving `form`. Options:
 * - config(): the page's current config (API URL, revision, choice mode);
 * - onSaved(): after a save, once everything is stored;
 * - onDefinitive(outcome): a refusal that needs the page reloaded;
 * - savedValues: Option IDs to show as just saved from the start (after
 *   the page was refreshed in place).
 * Returns { flush, unsaved, busy, stop }.
 */
export function createAutosave(form, options) {
  const allowNoOption = () => options.config().allowNoOption;
  // What is stored: the Option IDs of the Response, or `null` without one.
  let stored = options.config().answered ? checkedIds(form) : null;
  // Whether what is stored is the Preselection, not saved by the voter.
  let preselected = stored !== null && options.config().preselected;
  // Whether the voter has asked for anything yet (a change, or Save): until
  // then, an empty selection without a Response is no answer, not "no
  // Option".
  let touched = false;
  let inFlight = null;
  let debounce = null;
  let retryTimer = null;
  let failure = null;
  let saved = false;
  let stopped = false;
  const flashing = new Map();

  // --- The page's parts ---------------------------------------------------

  form.setAttribute("data-vota-autosave", "");
  const buttons = submitButtons(form);
  // Without a Response on a Ballot that allows no Option, Save stays: it
  // saves "no Option" as it stands. So it does while the Response is the
  // Preselection: it saves that as the voter's.
  function hideButtons() {
    const keep = (stored === null && allowNoOption()) || preselected;
    buttons.forEach((button) => {
      button.hidden = !keep;
    });
  }
  hideButtons();
  const box = element("div", "vota-autosave");
  box.setAttribute("data-state", "idle");
  const line = element("p", "vota-autosave-status");
  line.setAttribute("role", "status");
  line.setAttribute("aria-live", "polite");
  line.setAttribute("aria-atomic", "true");
  const icon = element("span", "vota-autosave-icon");
  icon.setAttribute("aria-hidden", "true");
  const text = element("span", "vota-autosave-text");
  const reason = element("span", "vota-autosave-reason");
  line.append(icon, text, reason);
  const retry = element("button", "vota-retry vota-secondary", "Retry now");
  retry.type = "button";
  retry.hidden = true;
  retry.addEventListener("click", () => retryNow(true));
  box.append(line, retry);
  if (buttons[0] && buttons[0].parentNode) {
    buttons[0].parentNode.insertBefore(box, buttons[0]);
  } else {
    form.appendChild(box);
  }

  form.addEventListener("change", onChange);

  // --- Feedback -----------------------------------------------------------

  function setOptionState(input, state) {
    const card = optionBox(input);
    if (!card) {
      return;
    }
    let badge = card.querySelector(":scope > .vota-option-state");
    if (!state) {
      card.removeAttribute("data-vota-save");
      if (badge) {
        badge.remove();
      }
      return;
    }
    if (card.getAttribute("data-vota-save") !== state) {
      card.setAttribute("data-vota-save", state);
    }
    if (!badge) {
      badge = element("span", "vota-option-state");
      badge.setAttribute("aria-hidden", "true");
      card.appendChild(badge);
    }
    if (badge.textContent !== OPTION_TEXT[state]) {
      badge.textContent = OPTION_TEXT[state];
    }
  }

  function flash(input) {
    const card = optionBox(input);
    window.clearTimeout(flashing.get(input));
    if (card && card.getAttribute("data-vota-save") === "saved") {
      // Restart the fade.
      card.removeAttribute("data-vota-save");
      void card.offsetWidth;
    }
    flashing.set(
      input,
      window.setTimeout(() => {
        flashing.delete(input);
        render();
      }, SAVED_FOR),
    );
  }

  function setStatus(state, words, why) {
    if (box.getAttribute("data-state") !== state) {
      box.setAttribute("data-state", state);
    }
    if (icon.textContent !== ICON[state]) {
      icon.textContent = ICON[state];
    }
    if (text.textContent !== words) {
      text.textContent = words;
    }
    if (reason.textContent !== why) {
      reason.textContent = why;
    }
    retry.hidden = state !== "failed";
  }

  /** The selection the page shows, to compare with what is stored: the
   * checked Options. Nothing checked without a Response is no answer
   * (`null`), unless the Ballot allows no Option and the voter has asked
   * for something: then it is `[]`, the answer "no Option". */
  function wanted() {
    const ui = checkedIds(form);
    if (stored === null && ui.length === 0 && !(touched && allowNoOption())) {
      return null;
    }
    return ui;
  }

  function render() {
    const ui = wanted();
    const dirty = !same(ui, stored);
    const invalid = dirty && ui.length === 0 && !allowNoOption();
    const shown = stored || [];
    for (const input of optionInputs(form)) {
      const changed = input.checked !== shown.includes(input.value);
      let state = null;
      if (changed) {
        state = failure || invalid ? "failed" : "pending";
      } else if (flashing.has(input)) {
        state = "saved";
      }
      setOptionState(input, state);
    }
    if (failure && (dirty || inFlight)) {
      const retrying = failure.transient && !failure.gaveUp;
      setStatus("failed", retrying ? "Not saved, retrying…" : "Not saved.", failure.reason);
    } else if (invalid) {
      setStatus("invalid", "Not saved: select at least one Option.", "Your saved answer is unchanged.");
    } else if (dirty || inFlight) {
      setStatus("pending", "Saving…", "");
    } else if (saved) {
      setStatus("saved", "Saved", "");
    } else {
      setStatus("idle", "Changes are saved automatically.", "");
    }
  }

  // --- Saving -------------------------------------------------------------

  function onChange(event) {
    const input = event.target;
    if (stopped || !input || input.name !== "option") {
      return;
    }
    saved = false;
    touched = true;
    window.clearTimeout(debounce);
    // A change while a retry waits: the newest state goes out soon instead.
    window.clearTimeout(retryTimer);
    retryTimer = null;
    debounce = window.setTimeout(
      () => {
        debounce = null;
        flush();
      },
      options.config().multiple ? DEBOUNCE_MULTIPLE : 0,
    );
    render();
  }

  /** Sends the current selection now, unless a request is in flight (the
   * newest selection then goes out right after it). Also Save's action:
   * the selection as it stands is what the voter wants saved. */
  function flush() {
    if (stopped || inFlight) {
      return;
    }
    touched = true;
    window.clearTimeout(debounce);
    debounce = null;
    window.clearTimeout(retryTimer);
    retryTimer = null;
    const ids = wanted();
    if (same(ids, stored) && !preselected) {
      failure = null;
      render();
      return;
    }
    if (ids.length === 0 && !allowNoOption()) {
      render();
      return;
    }
    send(ids);
  }

  function send(ids) {
    inFlight = ids;
    render();
    putResponse(options.config(), ids).then((result) => {
      inFlight = null;
      if (result.ok) {
        succeeded(ids);
      } else if (!result.transient) {
        failure = { transient: false, reason: result.reason };
        render();
        if (result.reload) {
          options.onDefinitive(result);
        }
      } else {
        failedForNow(result);
      }
    });
  }

  function succeeded(ids) {
    const before = stored || [];
    stored = ids;
    preselected = false;
    failure = null;
    hideButtons();
    for (const input of optionInputs(form)) {
      const wasChanged = before.includes(input.value) !== ids.includes(input.value);
      const nowStored = input.checked === ids.includes(input.value);
      if (wasChanged && nowStored) {
        flash(input);
      }
    }
    if (stopped) {
      render();
      return;
    }
    if (!same(wanted(), stored)) {
      // Newer changes: they go next, so this is not "Saved" yet.
      if (debounce === null) {
        flush();
      } else {
        render();
      }
      return;
    }
    saved = true;
    render();
    options.onSaved();
  }

  function failedForNow(result) {
    if (!failure || !failure.transient) {
      failure = { transient: true, firstAt: Date.now(), attempts: 0 };
    }
    failure.attempts += 1;
    failure.reason = result.reason;
    failure.gaveUp = false;
    if (debounce === null && !stopped) {
      let delay = failure.attempts <= BACKOFF.length ? BACKOFF[failure.attempts - 1] : BACKOFF_STEADY;
      if (result.retryAfter !== null) {
        delay = Math.max(delay, result.retryAfter);
      }
      if (Date.now() + delay - failure.firstAt > RETRY_FOR) {
        failure.gaveUp = true;
      } else {
        retryTimer = window.setTimeout(() => {
          retryTimer = null;
          flush();
        }, delay);
      }
    }
    render();
  }

  /** Retries a failed save now: by hand (which restarts the backoff), or
   * because the connection or the tab is back. */
  function retryNow(byHand) {
    if (stopped || !failure || inFlight) {
      return;
    }
    if (byHand && failure.transient) {
      failure.firstAt = Date.now();
      failure.attempts = 0;
    }
    flush();
  }

  function onBack() {
    if (failure && failure.transient && document.visibilityState !== "hidden") {
      retryNow(false);
    }
  }
  window.addEventListener("online", onBack);
  document.addEventListener("visibilitychange", onBack);

  (options.savedValues || []).forEach((value) => {
    const input = optionInputs(form).find((candidate) => candidate.value === value);
    if (input) {
      flash(input);
    }
  });
  // After a refresh in place following a save (even of "no Option").
  saved = Array.isArray(options.savedValues);
  render();

  return {
    form,
    flush,
    /** Whether anything shown is not stored yet (pending, waiting to
     * retry, failed, or a selection that can't be sent). */
    unsaved() {
      return (
        inFlight !== null ||
        debounce !== null ||
        retryTimer !== null ||
        failure !== null ||
        !same(wanted(), stored)
      );
    },
    busy() {
      return inFlight !== null;
    },
    /** Stops for good (at the deadline, or when the page is replaced). */
    stop() {
      stopped = true;
      window.clearTimeout(debounce);
      window.clearTimeout(retryTimer);
      debounce = null;
      retryTimer = null;
      window.removeEventListener("online", onBack);
      document.removeEventListener("visibilitychange", onBack);
      form.removeEventListener("change", onChange);
    },
  };
}
