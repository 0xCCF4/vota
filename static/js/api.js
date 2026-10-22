// Vota: saving a Response through the voter API (PUT, JSON), and what its
// answer means for the voter.
//
// `putResponse` never rejects. It resolves to one outcome:
// - { ok: true, body }: recorded;
// - { ok: false, transient: true, … }: worth retrying (no connection, a
//   timeout, a 5xx, or 429 with its Retry-After in `retryAfter`, ms);
// - { ok: false, transient: false, … }: definitive (409, 404, 422, other
//   4xx); `reload` says whether the page must be reloaded to show the
//   Ballot as it now stands.
// Each failure has a `title` and `message` (for a dialog) and a short
// `reason` (for the inline status line).

/** How long a save may take before it counts as a connection problem. */
export const SAVE_TIMEOUT = 20 * 1000;

export function putResponse(config, ids) {
  const controller = typeof AbortController === "function" ? new AbortController() : null;
  const timer = window.setTimeout(() => {
    if (controller) {
      controller.abort();
    }
  }, SAVE_TIMEOUT);
  return window
    .fetch(config.api, {
      method: "PUT",
      headers: { "Content-Type": "application/json", Accept: "application/json" },
      body: JSON.stringify({ option_ids: ids, revision: config.revision }),
      credentials: "same-origin",
      cache: "no-store",
      signal: controller ? controller.signal : undefined,
    })
    .then((response) =>
      response
        .json()
        .catch(() => ({}))
        .then((body) => outcome(response.status, body || {}, response.headers.get("Retry-After"))),
    )
    .catch(() => failure(true, "Not saved", "Could not save, check your connection.", "No connection to the server."))
    .finally(() => window.clearTimeout(timer));
}

function failure(transient, title, message, reason, extra) {
  return Object.assign({ ok: false, transient, reload: false, title, message, reason, retryAfter: null }, extra);
}

/** Retry-After (seconds or an HTTP date) in milliseconds, or null. */
function retryAfterMs(header) {
  if (!header) {
    return null;
  }
  const seconds = Number(header);
  if (Number.isFinite(seconds) && seconds >= 0) {
    return seconds * 1000;
  }
  const date = Date.parse(header);
  return isNaN(date) ? null : Math.max(0, date - Date.now());
}

/** What the voter API's answer means for the voter. */
export function outcome(status, body, retryAfter) {
  if (status >= 200 && status < 300) {
    return { ok: true, body };
  }
  const error = body.error;
  if (status === 409 && error === "voting_closed") {
    return failure(false, "Voting has closed", "Voting has closed. Your answer could not be accepted.", "Voting has closed.", {
      reload: true,
    });
  }
  if (status === 409 && error === "already_answered") {
    return failure(
      false,
      "Already answered",
      "This Ballot has already been answered. Your answer cannot be changed.",
      "Already answered.",
      { reload: true },
    );
  }
  if (status === 409 && error === "ballot_changed") {
    return failure(
      false,
      "This Ballot was changed",
      "This Ballot was changed by its organiser, so your answer was not saved. Please check it and answer again.",
      "The Ballot was changed.",
      { reload: true },
    );
  }
  if (status === 404) {
    return failure(false, "Not found", "This Ballot could not be found. It may have been deleted.", "Not found.", {
      reload: true,
    });
  }
  if (status === 422) {
    const detail = typeof body.message === "string" ? body.message : "the answer is not valid";
    return failure(false, "Not saved", "Could not save: " + detail + ".", "Could not save: " + detail + ".");
  }
  if (status === 429) {
    return failure(true, "Not saved", "Too many requests, try again later.", "Too many requests.", {
      retryAfter: retryAfterMs(retryAfter),
    });
  }
  if (status >= 500) {
    return failure(true, "Not saved", "Could not save, please try again.", "The server could not save it.");
  }
  return failure(false, "Not saved", "Could not save, please try again.", "Could not save.");
}
