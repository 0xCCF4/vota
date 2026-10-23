// Vota: what the server told the voting page's script. The app inserts
// `<script type="module" id="vota-enhancement" data-…>` into every voting
// page (see src/web/enhancement.rs); modules have no
// document.currentScript, so the tag is found by its id. Never inline
// script: the Content Security Policy forbids it.

/** The id of the app's own <script> tag. */
export const TAG_ID = "vota-enhancement";

/** The app's <script> tag in `doc`, if it carries the script's data. */
export function findTag(doc) {
  const tag = doc.getElementById(TAG_ID);
  return tag && tag.dataset && tag.dataset.page ? tag : null;
}

/** What the server told us, from the tag's data-* attributes. */
export function readConfig(tag) {
  const data = tag.dataset;
  const serverNow = Date.parse(data.now);
  return {
    api: data.api,
    page: data.page,
    deadline: Date.parse(data.deadline),
    // Server "now" minus client "now": corrects a skewed client clock.
    offset: isNaN(serverNow) ? 0 : serverNow - Date.now(),
    open: data.open === "true",
    // The Ballot's revision as rendered: a Response is recorded only if the
    // Ballot still stands as the voter saw it.
    revision: Number(data.revision),
    multiple: data.choiceMode === "multiple",
    editable: data.submissionMode === "editable",
    needsConfirmation: data.needsConfirmation === "true",
    // Whether a Response may select no Option at all ("none of these").
    allowNoOption: data.allowNoOption === "true",
    // Whether the Ballot has a Response: an empty selection on the page is
    // then the voter's answer of no Option, not "no answer yet".
    answered: data.answered === "true",
    // Whether that Response is the admin's Preselection, which the voter
    // hasn't saved (or changed) yet: saving it as it stands makes it theirs.
    preselected: data.preselected === "true",
  };
}

/** "Now" by the server's clock. */
export function now(config) {
  return Date.now() + config.offset;
}
