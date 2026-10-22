// Vota: the voting page's dialog: the Final Ballot's confirmation step
// (as on the no-JS confirmation page), "Saving…" while a Final Response is
// sent, and results the voter must acknowledge ("Saved ✓", or why the
// answer could not be accepted). One <dialog class="vota-dialog">, reused.

import { element } from "./dom.js";

/** Whether this browser can show the dialog (and save without a reload). */
export const canDialog =
  typeof HTMLDialogElement === "function" &&
  typeof document.createElement("dialog").showModal === "function" &&
  typeof window.fetch === "function";

let dialog = null;
// The open dialog's pending outcome: what to do when the voter answers it
// (a button, or Escape). Settled at most once, synchronously, so a late
// "close" event of one dialog can never act on the next one.
let session = null;

function node() {
  if (dialog) {
    return dialog;
  }
  dialog = element("dialog", "vota-dialog");
  dialog.setAttribute("aria-labelledby", "vota-dialog-title");
  dialog.setAttribute("aria-describedby", "vota-dialog-message");
  // Focusable itself, for while it has no buttons ("Saving…").
  dialog.setAttribute("tabindex", "-1");
  // Escape: the dialog's "cancel" answer; while saving, nothing.
  dialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    settle("cancel");
  });
  // Closed by the browser despite that: also "cancel" (but never act on a
  // dialog that has been reopened since).
  dialog.addEventListener("close", () => {
    if (!dialog.open) {
      settle("cancel");
    }
  });
  return dialog;
}

function settle(result) {
  const pending = session;
  session = null;
  if (pending) {
    pending(result);
  }
}

/** Whether the dialog is open, and in which state. */
export function dialogState() {
  return dialog && dialog.open ? dialog.getAttribute("data-state") : null;
}

/** Answers an open confirmation with "cancel" (e.g. at the deadline). */
export function cancelConfirmation() {
  if (dialogState() === "confirm") {
    settle("cancel");
  }
}

export function closeDialog() {
  session = null;
  if (dialog && dialog.open) {
    dialog.close();
  }
}

/** Fills the dialog (opening it if needed): a title, a message, extra
 * content, buttons, and what to do once the voter answers it. */
function fill(state, title, message, extra, buttons, onAnswer) {
  const box = node();
  session = onAnswer || null;
  box.setAttribute("data-state", state);
  box.replaceChildren();
  const heading = element("h2", "vota-dialog-title");
  heading.id = "vota-dialog-title";
  if (title.node) {
    heading.appendChild(title.node);
  } else {
    heading.textContent = title;
  }
  box.appendChild(heading);
  const text = element("p", "vota-dialog-message", message);
  text.id = "vota-dialog-message";
  text.setAttribute("role", "status");
  text.setAttribute("aria-live", "polite");
  box.appendChild(text);
  (extra || []).forEach((child) => box.appendChild(child));
  if (buttons.length) {
    const actions = element("div", "vota-dialog-actions");
    buttons.forEach((button) => actions.appendChild(button));
    box.appendChild(actions);
  }
  if (!box.isConnected) {
    document.body.appendChild(box);
  }
  if (!box.open) {
    box.showModal();
  }
}

/** A dialog button answering the dialog with `result`. */
function button(text, className, result) {
  const node = element("button", className, text);
  node.type = "button";
  node.addEventListener("click", () => settle(result));
  return node;
}

/** The Final confirmation step for the chosen Options' labels (none: "no
 * Option", on a Ballot that allows it). `onAnswer(true)` to go ahead (the
 * dialog stays open, to show "Saving…" next), `onAnswer(false)` once
 * closed. */
export function confirmChoice(labels, onAnswer) {
  const list = element("ul", "vota-selection");
  labels.forEach((label) => list.appendChild(element("li", null, label)));
  const chosen = labels.length ? [list] : [];
  const warning = element("p", "vota-warning");
  warning.appendChild(element("strong", null, "You cannot change your answer after you confirm it."));
  const confirm = button("Confirm and submit", "vota-primary", "confirm");
  const back = button("Go back and choose again", "vota-secondary", "cancel");
  const message = labels.length ? "You chose:" : "You chose no option.";
  fill("confirm", "Confirm your answer", message, [...chosen, warning], [confirm, back], (result) => {
    if (result === "confirm") {
      onAnswer(true);
    } else {
      closeDialog();
      onAnswer(false);
    }
  });
  // Start on the safe choice, so a stray Enter doesn't lock the answer in.
  back.focus();
}

/** "Saving…", without buttons. */
export function showSaving() {
  fill("saving", "Saving your answer", "Saving…", [], []);
  node().focus();
}

/** A result with one button ("OK", or `buttonText`); `after` runs once the
 * voter has acknowledged it and the dialog is closed. */
export function showResult(state, title, message, after, buttonText) {
  const ok = button(buttonText || "OK", "vota-primary", "ok");
  fill(state, title, message, [], [ok], () => {
    closeDialog();
    after();
  });
  ok.focus();
}

/** The "Saved ✓" title, with the mark hidden from screen readers. */
export function savedTitle() {
  const check = document.createDocumentFragment();
  check.appendChild(document.createTextNode("Saved "));
  const mark = element("span", null, "✓");
  mark.setAttribute("aria-hidden", "true");
  check.appendChild(mark);
  return { node: check };
}
