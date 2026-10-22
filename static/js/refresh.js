// Vota: refreshing the voting page in place. After a save the page fetches
// its own server rendering and swaps in the fresh <body>, without a
// reload, so the Ballot Status and notices follow the stored Response. The
// stylesheets must be the same; otherwise (and on any problem) the caller
// reloads instead.

import { findTag, readConfig } from "./config.js";

/** Fetches the page and swaps in its body. Resolves to the fresh page's
 * config; rejects if the page can't be swapped in place. `stillWanted()`
 * is asked again right before swapping (e.g. no newer unsaved change). */
export function refreshPage(config, stillWanted) {
  return window
    .fetch(config.page, {
      headers: { Accept: "text/html" },
      credentials: "same-origin",
      cache: "no-store",
    })
    .then((response) => {
      if (!response.ok) {
        throw new Error("status " + response.status);
      }
      return response.text();
    })
    .then((html) => {
      const fresh = new DOMParser().parseFromString(withOwnNonce(html), "text/html");
      const tag = findTag(fresh);
      if (!tag || styles(fresh) !== styles(document) || !fresh.body) {
        throw new Error("the page changed");
      }
      if (!stillWanted()) {
        return null;
      }
      document.title = fresh.title;
      const nodes = Array.from(fresh.body.childNodes).map((node) => document.adoptNode(node));
      document.body.replaceChildren(...nodes);
      return readConfig(tag);
    });
}

/** The fresh page's markup with the nonce of its inline styles (each
 * response has its own) replaced by this page's: the parsed copy falls
 * under this page's Content Security Policy, which would refuse the new
 * nonce and report a violation. A fresh page with nonce'd styles where this
 * one has none has changed its styles: a reload. (Matches the double-quoted
 * `nonce="…"` of the built-in templates.) */
function withOwnNonce(html) {
  const own = document.querySelector("style[nonce]");
  const nonce = own ? own.nonce : "";
  if (!nonce) {
    if (/<style\b[^>]*\snonce="/i.test(html)) {
      throw new Error("the page changed");
    }
    return html;
  }
  return html.replace(/(<style\b[^>]*\snonce=)"[^"]*"/gi, (_, start) => start + '"' + nonce + '"');
}

/** The page's stylesheets and inline styles, to compare two pages. */
function styles(doc) {
  return Array.from(doc.querySelectorAll('link[rel="stylesheet"], style'))
    .map((node) => node.getAttribute("href") || node.textContent)
    .join("\n");
}
