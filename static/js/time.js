// Vota: the Voting Deadline in the voter's own time zone, and the
// countdown to it.
//
// Every <time datetime> on the page is rewritten into a short, friendly
// local form ("Today, 18:00", "Tomorrow, 09:30", "Tue 1 Dec, 13:00", with
// the year only when it isn't this year), with the precise date and zone as
// its tooltip. The countdown goes into the template's placeholder (an
// element with the class "vota-countdown"), if it has one; the deadline is
// watched either way, so the form still closes on time without one.

import { now } from "./config.js";

const MINUTE = 60 * 1000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

function pad(number) {
  return (number < 10 ? "0" : "") + number;
}

/** The local calendar day of `date`, as a day count. */
function dayNumber(date) {
  return Math.round(Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / DAY);
}

/** `instant` in the voter's local time, short: "Today, 18:00",
 * "Tomorrow, 09:30", "Tue 1 Dec, 13:00" or "Tue 1 Dec 2027, 13:00". */
export function friendlyDate(instant, nowMs) {
  const date = new Date(instant);
  const today = new Date(nowMs);
  const time = pad(date.getHours()) + ":" + pad(date.getMinutes());
  const days = dayNumber(date) - dayNumber(today);
  if (days === 0) {
    return "Today, " + time;
  }
  if (days === 1) {
    return "Tomorrow, " + time;
  }
  let day = WEEKDAYS[date.getDay()] + " " + date.getDate() + " " + MONTHS[date.getMonth()];
  if (date.getFullYear() !== today.getFullYear()) {
    day += " " + date.getFullYear();
  }
  return day + ", " + time;
}

/** The precise form for the tooltip: the full date with the zone. */
const preciseFormat = (() => {
  if (typeof Intl !== "object" || !Intl.DateTimeFormat) {
    return null;
  }
  // The page is in English: the voter's English variant if they have one,
  // otherwise British English (as the server's own format reads).
  const language = navigator.language || "";
  const locale = /^en(-|$)/i.test(language) ? language : "en-GB";
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

/** Every <time datetime> in the voter's local time zone (once each). */
export function localizeTimes(config) {
  for (const time of document.querySelectorAll("time[datetime]")) {
    const instant = Date.parse(time.getAttribute("datetime"));
    if (isNaN(instant) || time.hasAttribute("data-vota-local")) {
      continue;
    }
    time.title = preciseFormat ? preciseFormat.format(new Date(instant)) : new Date(instant).toString();
    time.textContent = friendlyDate(instant, now(config));
    time.setAttribute("data-vota-local", "");
  }
}

function plural(count, unit) {
  return count + " " + unit + (count === 1 ? "" : "s");
}

/** "2 days, 23 hours", "2 hours, 5 minutes", "4 minutes, 9 seconds", … */
export function remainingText(ms) {
  const seconds = Math.floor(ms / 1000);
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = seconds % 60;
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

let timer = null;

/** Counts down to the deadline in the template's placeholder(s), if any,
 * and calls `onClosed` once it has passed. */
export function startCountdown(config, onClosed) {
  stopCountdown();
  if (!config.open || isNaN(config.deadline)) {
    return;
  }
  const tick = () => {
    const placeholders = document.querySelectorAll(".vota-countdown");
    const remaining = config.deadline - now(config);
    if (remaining <= 0) {
      timer = null;
      placeholders.forEach((node) => {
        node.textContent = "Voting has closed.";
      });
      onClosed();
      return;
    }
    const text = "in " + remainingText(remaining);
    placeholders.forEach((node) => {
      if (node.textContent !== text) {
        node.textContent = text;
      }
    });
    // Wake up just after the shown value changes: every second in the last
    // hour, every minute before.
    const step = remaining > HOUR ? MINUTE : 1000;
    timer = window.setTimeout(tick, (remaining % step) + 20);
  };
  tick();
}

export function stopCountdown() {
  if (timer !== null) {
    window.clearTimeout(timer);
    timer = null;
  }
}
