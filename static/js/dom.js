// Vota: small DOM helpers of the voting page's script, and how it finds
// the no-JS voting form it builds on: a method="post" form whose "option"
// inputs are radio buttons or checkboxes, each with a <label for>. That is
// all the script needs from a Theme's templates.

/** A new element with an optional class and text. */
export function element(tag, className, text) {
  const node = document.createElement(tag);
  if (className) {
    node.className = className;
  }
  if (text !== undefined) {
    node.textContent = text;
  }
  return node;
}

/** The no-JS voting form, if the page has one. */
export function votingForm() {
  for (const form of document.querySelectorAll("form")) {
    if (isVotingForm(form)) {
      return form;
    }
  }
  return null;
}

/** Whether `form` posts checkable "option" inputs. */
export function isVotingForm(form) {
  if ((form.getAttribute("method") || "").toLowerCase() !== "post") {
    return false;
  }
  const inputs = optionInputs(form);
  return inputs.length > 0 && inputs.every((input) => input.type === "radio" || input.type === "checkbox");
}

/** The form's "option" inputs, in page order. */
export function optionInputs(form) {
  return Array.from(form.querySelectorAll('input[name="option"]'));
}

/** The IDs of the checked Options, in page order. */
export function checkedIds(form) {
  return optionInputs(form)
    .filter((input) => input.checked)
    .map((input) => input.value);
}

/** The Option's visible label. */
export function labelOf(input) {
  const labels = input.labels;
  if (labels && labels.length) {
    return labels[0].textContent.trim();
  }
  return input.value;
}

/** The element that stands for one Option on the page (its card): the
 * input's closest `.option`, or else its parent. */
export function optionBox(input) {
  return input.closest(".option") || input.parentElement;
}

/** The form's submit buttons. */
export function submitButtons(form) {
  return Array.from(form.querySelectorAll('button[type="submit"], button:not([type]), input[type="submit"]'));
}

/** A polite live region for announcements outside any dialog, never shown. */
export function liveRegion() {
  const live = element("p", "vota-visually-hidden");
  live.setAttribute("aria-live", "polite");
  live.setAttribute("aria-atomic", "true");
  return live;
}
