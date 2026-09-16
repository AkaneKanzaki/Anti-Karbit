/**
 * AntiKarbit dashboard — client helper.
 *
 * HTMX handles the request/response parts of the UI (bot toggle, statistics,
 * settings). This file only covers what HTML attributes cannot express:
 * tab switching, the mobile sidebar, console behaviour, and the live event
 * stream.
 *
 * The stream is consumed with a single EventSource rather than the HTMX SSE
 * extension. Both event types arrive on the same connection, and claim events
 * carry JSON that has to be formatted into markup before it reaches the DOM.
 */

document.addEventListener("DOMContentLoaded", () => {
  /* ------------------------------------------------------------------ Tabs */

  const TABS = {
    dashboard: {
      title: "Dashboard",
      desc: "Bot status, statistics, and recent claim activity.",
    },
    logs: {
      title: "Logs",
      desc: "Live system output and Telegram connection activity.",
    },
    settings: {
      title: "Settings",
      desc: "Claim command, group filters, and image matching thresholds.",
    },
  };

  const navItems = Array.from(document.querySelectorAll(".nav-item"));
  const panes = Array.from(document.querySelectorAll(".pane"));
  const titleEl = document.getElementById("current-tab-title");
  const descEl = document.getElementById("current-tab-desc");

  function switchTab(id) {
    navItems.forEach((btn) => btn.classList.toggle("is-active", btn.dataset.tab === id));
    panes.forEach((pane) => pane.classList.toggle("is-hidden", pane.id !== `pane-${id}`));

    const meta = TABS[id];
    if (meta) {
      if (titleEl) titleEl.textContent = meta.title;
      if (descEl) descEl.textContent = meta.desc;
    }

    closeSidebar();
  }

  navItems.forEach((btn) => {
    btn.addEventListener("click", () => {
      if (btn.dataset.tab) switchTab(btn.dataset.tab);
    });
  });

  /* --------------------------------------------------------------- Sidebar */

  const sidebar = document.getElementById("sidebar");
  const overlay = document.getElementById("sidebar-overlay");
  const btnOpen = document.getElementById("btn-hamburger-open");
  const btnClose = document.getElementById("btn-hamburger");

  function openSidebar() {
    sidebar?.classList.add("is-open");
    overlay?.classList.add("is-open");
  }

  function closeSidebar() {
    sidebar?.classList.remove("is-open");
    overlay?.classList.remove("is-open");
  }

  btnOpen?.addEventListener("click", openSidebar);
  btnClose?.addEventListener("click", closeSidebar);
  overlay?.addEventListener("click", closeSidebar);

  /* --------------------------------------------------------------- Console */

  const terminal = document.getElementById("log-terminal");
  const autoScroll = document.getElementById("chk-autoscroll");
  const btnClear = document.getElementById("btn-clear-logs");

  /** Keep the DOM small so a long-running session does not grow unbounded. */
  const MAX_LINES = 300;
  const TRIM_TO = 250;

  function trimConsole() {
    if (!terminal) return;
    while (terminal.children.length > MAX_LINES) {
      terminal.removeChild(terminal.firstChild);
    }
  }

  function scrollConsole() {
    if (terminal && autoScroll?.checked) {
      terminal.scrollTop = terminal.scrollHeight;
    }
  }

  btnClear?.addEventListener("click", () => {
    if (!terminal) return;
    terminal.replaceChildren();
    appendLog({ message: "[system] Console cleared.", level: "info" });
  });

  function appendLog(entry) {
    if (!terminal) return;

    const line = document.createElement("div");
    line.className = "log-line";
    if (entry.level && entry.level !== "info") {
      line.classList.add(`is-${entry.level}`);
    }
    line.textContent = entry.message ?? "";

    terminal.appendChild(line);
    trimConsole();
    scrollConsole();
  }

  /* ------------------------------------------------------------ Claim feed */

  const feed = document.getElementById("feed-list");
  const feedCount = document.getElementById("feed-count");
  let feedTotal = 0;

  function formatTime(seconds) {
    if (!seconds) return "";
    const d = new Date(seconds * 1000);
    return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  }

  function appendClaim(event) {
    if (!feed) return;

    const success = event.type === "claim_result";
    const name = event.character_name || (success ? "Unknown" : "Claim failed");

    const item = document.createElement("div");
    item.className = `feed-item ${success ? "is-ok" : "is-fail"}`;

    const head = document.createElement("div");
    head.className = "feed-item-head";

    const title = document.createElement("span");
    title.className = "feed-name";
    title.textContent = name;

    const meta = document.createElement("span");
    meta.className = "feed-meta";
    meta.textContent = success && event.confidence ? `${event.confidence}%` : "failed";

    head.append(title, meta);

    const detail = document.createElement("div");
    detail.className = "feed-meta";
    detail.textContent = [
      event.series,
      event.engine || event.source,
      formatTime(event.timestamp),
    ]
      .filter(Boolean)
      .join("  ·  ");

    item.append(head, detail);

    document.getElementById("feed-empty")?.remove();
    feed.prepend(item);

    while (feed.children.length > 50) {
      feed.removeChild(feed.lastChild);
    }

    feedTotal += 1;
    if (feedCount) feedCount.textContent = String(feedTotal);
  }

  /* ------------------------------------------------------------ Live stream */

  function connectStream() {
    const source = new EventSource("/api/events");

    source.addEventListener("log", (e) => {
      try {
        appendLog(JSON.parse(e.data));
      } catch {
        /* Ignore malformed frames rather than tearing down the stream. */
      }
    });

    source.addEventListener("claim_event", (e) => {
      try {
        appendClaim(JSON.parse(e.data));
      } catch {
        /* Same as above. */
      }
    });

    // EventSource reconnects on its own; this only surfaces the state.
    source.addEventListener("error", () => {
      if (source.readyState === EventSource.CLOSED) {
        appendLog({ message: "[system] Event stream closed. Reload to reconnect.", level: "warning" });
      }
    });
  }

  connectStream();
});
