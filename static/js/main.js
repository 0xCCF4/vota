// Vota: the voting page's enhancement, entry module. Plain JavaScript as
// native ES modules, no build step, no third-party code, no requests to any
// other host. Browsers without module support get the plain page.
//
// The app includes it on every voting page (see src/web/enhancement.rs)
// as <script type="module" id="vota-enhancement">, with what it needs in
// data-* attributes on that tag. Without it (or without JavaScript) the
// page works as a plain form (post/redirect/get). With it:
// - the Voting Deadline is shown in the voter's local time zone, with a
//   countdown in the template's placeholder; at the deadline the form is
//   disabled and the page reloads to show the closed view (time.js);
// - an Editable Ballot saves every change at once, without a Submit button
//   (autosave.js);
// - a Final Ballot keeps its explicit Submit: it asks for confirmation in a
//   dialog, then saves with "Saving…" and "Saved ✓" (dialog.js);
// - every save goes through the voter API with the Ballot's revision as
//   rendered (api.js), so an admin edit since the page was loaded refuses
//   the Response ("This Ballot was changed") instead of recording it
//   against wording the voter never saw; after a save the page refreshes
//   in place (refresh.js).
//
// It relies on nothing in the page but the no-JS form (a method="post"
// form with "option" inputs and their <label for>s), <time datetime>
// elements for local times and, optionally, a ".vota-countdown"
// placeholder, so it works with any Theme's templates.

import { createAutosave } from "./autosave.js";
import { findTag, readConfig } from "./config.js";
import * as dialog from "./dialog.js";
import { checkedIds, isVotingForm, labelOf, liveRegion, optionInputs, submitButtons, votingForm } from "./dom.js";
import { putResponse } from "./api.js";
import { refreshPage } from "./refresh.js";
import { localizeTimes, startCountdown, stopCountdown } from "./time.js";

/** How long "Voting has closed" shows before the page reloads. */
const CLOSE_RELOAD_DELAY = 1500;

const tag = findTag(document);
if (tag) {
  start(readConfig(tag));
}

function start(initialConfig) {
  let config = initialConfig;
  let autosave = null;
  // Until the first autosave, the page still shows the Ballot as
  // Unanswered: refresh it in place once everything is saved.
  let needsRefresh = false;
  let finalSaving = false;
  let reloadWhenIdle = false;
  // The page's own reloads never ask the voter to confirm leaving.
  let leaving = false;

  // Announcements for screen readers outside the dialog (e.g. "Voting has
  // closed"): a polite live region, present from the start so changes to
  // it are read out.
  const live = liveRegion();

  function enhance(savedValues) {
    localizeTimes(config);
    startCountdown(config, votingClosed);
    const form = votingForm();
    if (form && config.open && config.editable && dialog.canDialog) {
      needsRefresh = needsRefresh || !config.answered;
      autosave = createAutosave(form, {
        config: () => config,
        onSaved: autosaved,
        onDefinitive: (result) => showResult("error", result.title, result.message, reload, "Reload the page"),
        savedValues,
      });
    }
  }

  /** The server-rendered page, freshly (never re-posting a form). */
  function reload() {
    leaving = true;
    window.location.replace(config.page);
  }

  function focusForm() {
    const form = votingForm();
    if (!form) {
      return;
    }
    const target =
      submitButtons(form).find((button) => !button.hidden) ||
      optionInputs(form).find((input) => input.checked) ||
      optionInputs(form)[0];
    if (target) {
      target.focus();
    }
  }

  /** A result dialog; once acknowledged: `after`, or a reload the deadline
   * asked for meanwhile, or back to the form. */
  function showResult(state, title, message, after, buttonText) {
    dialog.showResult(
      state,
      title,
      message,
      () => {
        if (reloadWhenIdle) {
          reload();
        } else if (after) {
          after();
        } else {
          focusForm();
        }
      },
      buttonText || (after === reload ? "Reload the page" : undefined),
    );
  }

  // --- The deadline -------------------------------------------------------

  /** The deadline has passed: lock the form, say so, then reload. */
  function votingClosed() {
    stopCountdown();
    config.open = false;
    if (autosave) {
      autosave.stop();
    }
    const form = votingForm();
    if (form) {
      for (const control of form.elements) {
        control.disabled = true;
      }
    }
    live.textContent = "Voting has closed.";
    dialog.cancelConfirmation();
    reloadOnceIdle();
  }

  function reloadOnceIdle() {
    if (dialog.dialogState()) {
      // A save or a result is showing: its answer reloads.
      reloadWhenIdle = true;
    } else if (finalSaving || (autosave && autosave.busy())) {
      window.setTimeout(reloadOnceIdle, 200);
    } else {
      window.setTimeout(reload, CLOSE_RELOAD_DELAY);
    }
  }

  // --- Saving ---------------------------------------------------------------

  /** Everything is autosaved: the first time, refresh the page in place, so
   * its status and notices show the Ballot as Answered. */
  function autosaved() {
    if (!needsRefresh || !autosave) {
      return;
    }
    const current = autosave;
    const focused = document.activeElement && document.activeElement.id;
    const values = checkedIds(current.form);
    refreshPage(config, () => autosave === current && !current.unsaved() && config.open)
      .then((fresh) => {
        if (!fresh) {
          return;
        }
        needsRefresh = false;
        current.stop();
        autosave = null;
        config = fresh;
        document.body.appendChild(live);
        enhance(values);
        const again = focused && document.getElementById(focused);
        if (again) {
          again.focus();
        }
      })
      .catch(reload);
  }

  /** The no-JS form's submit: an autosaved form saves now; a Final Ballot
   * asks for confirmation, then saves in place. */
  function onSubmit(event) {
    const form = event.target;
    if (!config.open || !isVotingForm(form)) {
      return;
    }
    event.preventDefault();
    if (autosave && autosave.form === form) {
      autosave.flush();
      return;
    }
    if (finalSaving) {
      return;
    }
    const chosen = optionInputs(form).filter((input) => input.checked);
    if (chosen.length === 0 && !config.allowNoOption) {
      showResult(
        "error",
        "Nothing selected",
        config.multiple ? "Select at least one Option." : "Select exactly one Option.",
        null,
      );
      return;
    }
    const ids = chosen.map((input) => input.value);
    if (!config.needsConfirmation) {
      saveFinal(ids);
      return;
    }
    dialog.confirmChoice(chosen.map(labelOf), (confirmed) => {
      if (confirmed && config.open) {
        // The dialog stays open and turns into "Saving…".
        saveFinal(ids);
      } else {
        dialog.closeDialog();
        focusForm();
      }
    });
  }

  function saveFinal(ids) {
    finalSaving = true;
    dialog.showSaving();
    putResponse(config, ids).then((result) => {
      finalSaving = false;
      if (result.ok) {
        const editable = result.body.submission_mode === "editable";
        showResult(
          "saved",
          dialog.savedTitle(),
          editable
            ? "Your answer has been saved. You can change it until voting closes."
            : "Your answer has been recorded.",
          refreshAfterFinal,
        );
      } else {
        showResult("error", result.title, result.message, result.reload ? reload : null);
      }
    });
  }

  function refreshAfterFinal() {
    refreshPage(config, () => true)
      .then((fresh) => {
        if (autosave) {
          autosave.stop();
          autosave = null;
        }
        config = fresh;
        document.body.appendChild(live);
        enhance();
        const heading =
          document.querySelector("main h1") || document.querySelector("h1") || document.querySelector("main");
        if (heading) {
          heading.setAttribute("tabindex", "-1");
          heading.focus();
        }
      })
      .catch(reload);
  }

  // --- Start ------------------------------------------------------------------

  document.body.appendChild(live);
  enhance();
  if (dialog.canDialog) {
    document.addEventListener("submit", onSubmit);
  }
  // Leaving with something not stored yet: the browser asks first.
  window.addEventListener("beforeunload", (event) => {
    if (!leaving && autosave && autosave.unsaved()) {
      event.preventDefault();
      event.returnValue = "";
    }
  });
}
