/**
 * AntiKarbit Web Dashboard Client Logic
 */

// ==========================================
// AUTH LAYER — Cek login sebelum apapun
// ==========================================
const loginOverlay  = document.getElementById("login-overlay");
const appContainer  = document.getElementById("app-container");
const loginForm     = document.getElementById("login-form");
const loginInput    = document.getElementById("login-password-input");
const loginErrorMsg = document.getElementById("login-error-msg");
const btnLoginLabel = document.getElementById("btn-login-label");
const btnLogout     = document.getElementById("btn-logout");

/** Tampilkan overlay login */
function showLoginOverlay() {
  loginOverlay.style.display = "flex";
  appContainer.style.display = "none";
  setTimeout(() => loginInput && loginInput.focus(), 200);
}

/** Tampilkan konten utama */
function showApp(isProtected) {
  loginOverlay.style.display = "none";
  appContainer.style.display = "flex";
  if (btnLogout) btnLogout.style.display = isProtected ? "block" : "none";
}

/** Animasi shake + pesan error */
function showLoginError(msg) {
  loginErrorMsg.textContent = msg;
  loginInput.classList.remove("shake");
  // Trigger reflow untuk restart animasi
  void loginInput.offsetWidth;
  loginInput.classList.add("shake");
  setTimeout(() => loginInput.classList.remove("shake"), 500);
}

/** Fetch dengan penanganan otomatis untuk 401, 403 (CSRF), dan 429 (Rate limit) */
async function authedFetch(url, options = {}) {
  const resp = await fetch(url, options);
  if (resp.status === 401) {
    showLoginOverlay();
    throw new Error("Sesi berakhir, silakan login kembali.");
  }
  if (resp.status === 429) {
    const data = await resp.json().catch(() => ({}));
    alert(data.error || "Terlalu banyak permintaan (Rate limit). Silakan tunggu sebentar.");
    throw new Error("Rate limit exceeded");
  }
  if (resp.status === 403) {
    const data = await resp.json().catch(() => ({}));
    alert(data.error || "Permintaan ditolak oleh server (CSRF protection).");
    throw new Error("Forbidden");
  }
  return resp;
}

// Proses submit form login
if (loginForm) {
  loginForm.addEventListener("submit", async (e) => {
    e.preventDefault();
    const password = loginInput.value;
    if (!password) return;

    btnLoginLabel.textContent = "⏳ Memeriksa...";
    loginErrorMsg.textContent = "";

    try {
      const resp = await fetch("/api/auth/login", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ password }),
      });
      const data = await resp.json();

      if (data.authenticated) {
        loginInput.value = "";
        showApp(true);
      } else {
        showLoginError(data.message || "Password salah.");
      }
    } catch (err) {
      showLoginError("Gagal menghubungi server. Coba lagi.");
    } finally {
      btnLoginLabel.textContent = "Masuk ke Dashboard";
    }
  });
}

// Proses tombol logout
if (btnLogout) {
  btnLogout.addEventListener("click", async () => {
    await fetch("/api/auth/logout", { method: "POST" });
    showLoginOverlay();
  });
}

// Cek status auth saat halaman dimuat
(async () => {
  try {
    const resp = await fetch("/api/auth/check");
    const data = await resp.json();
    if (data.authenticated) {
      showApp(data.protected);
    } else {
      showLoginOverlay();
    }
  } catch {
    // Jika server belum siap, tampilkan app saja (fallback aman)
    showApp(false);
  }
})();

document.addEventListener("DOMContentLoaded", () => {

  // ==========================================
  // 0. Mobile Sidebar Toggle (Hamburger Menu)
  // ==========================================
  const sidebar       = document.getElementById("sidebar");
  const sidebarOvl    = document.getElementById("sidebar-overlay");
  const btnHamburger  = document.getElementById("btn-hamburger");

  function openSidebar() {
    if (sidebar)    sidebar.classList.add("open");
    if (sidebarOvl) sidebarOvl.classList.add("visible");
    document.body.style.overflow = "hidden";
  }

  function closeSidebar() {
    if (sidebar)    sidebar.classList.remove("open");
    if (sidebarOvl) sidebarOvl.classList.remove("visible");
    document.body.style.overflow = "";
  }

  if (btnHamburger) {
    btnHamburger.addEventListener("click", () => {
      if (sidebar && sidebar.classList.contains("open")) {
        closeSidebar();
      } else {
        openSidebar();
      }
    });
  }

  if (sidebarOvl) {
    sidebarOvl.addEventListener("click", closeSidebar);
  }

  // Auto-close sidebar on nav item click (mobile)
  document.querySelectorAll(".nav-item").forEach(btn => {
    btn.addEventListener("click", () => {
      if (window.innerWidth <= 768) closeSidebar();
    });
  });

  // Close sidebar when resizing to desktop
  window.addEventListener("resize", () => {
    if (window.innerWidth > 768) closeSidebar();
  });


  // Navigation tabs state
  const tabButtons = document.querySelectorAll(".nav-item");
  const tabPanes = document.querySelectorAll(".tab-pane");
  const currentTabTitle = document.getElementById("current-tab-title");
  const currentTabDesc = document.getElementById("current-tab-desc");

  const tabMeta = {
    dashboard: {
      title: "Dashboard Overview",
      desc: "Monitor performa klaim otomatis & aktivitas grup secara real-time",
    },
    tester: {
      title: "Waifu Vision Tester",
      desc: "Uji coba identifikasi gambar karakter secara instan tanpa perlu grup Telegram",
    },
    logs: {
      title: "Live Console Terminal",
      desc: "Pantau output dan aktivitas sistem langsung dari background runner",
    },
    settings: {
      title: "Pengaturan Bot",
      desc: "Sesuaikan trigger, perintah klaim, batas kemiripan, dan delay bot",
    },
  };

  tabButtons.forEach((btn) => {
    btn.addEventListener("click", () => {
      const tabKey = btn.getAttribute("data-tab");
      switchTab(tabKey);
    });
  });

  function switchTab(tabKey) {
    tabButtons.forEach((b) => b.classList.remove("active"));
    tabPanes.forEach((p) => p.classList.remove("active"));

    const targetBtn = document.querySelector(`.nav-item[data-tab="${tabKey}"]`);
    const targetPane = document.getElementById(`pane-${tabKey}`);

    if (targetBtn && targetPane) {
      targetBtn.classList.add("active");
      targetPane.classList.add("active");

      if (tabMeta[tabKey]) {
        currentTabTitle.textContent = tabMeta[tabKey].title;
        currentTabDesc.textContent = tabMeta[tabKey].desc;
      }
    }
  }

  // Quick link from dashboard to settings
  const btnGotoSettings = document.getElementById("btn-goto-settings");
  if (btnGotoSettings) {
    btnGotoSettings.addEventListener("click", () => switchTab("settings"));
  }

  // ==========================================
  // 1. Status Polling & Bot Toggle
  // ==========================================
  const statusDot = document.getElementById("status-dot");
  const statusText = document.getElementById("status-text");
  const btnToggleBot = document.getElementById("btn-toggle-bot");
  const btnToggleLabel = document.getElementById("btn-toggle-label");
  const sysRam = document.getElementById("sys-ram");

  const userName = document.getElementById("user-name");
  const userTag = document.getElementById("user-tag");

  const statDetected = document.getElementById("stat-detected");
  const statClaimed = document.getElementById("stat-claimed");
  const statFailed = document.getElementById("stat-failed");
  const statSuccessRate = document.getElementById("stat-success-rate");

  let isBotListening = true;

  async function fetchStatus() {
    try {
      const res = await authedFetch("/api/status");
      if (!res.ok) return;
      const data = await res.json();

      // Telegram Account
      if (data.user) {
        userName.textContent = data.user.name || "Telegram User";
        userTag.textContent = data.user.username ? `@${data.user.username}` : `ID: ${data.user.id}`;
      }

      // System Memory
      if (data.memory_mb) {
        sysRam.textContent = `${data.memory_mb} MB`;
      }

      // Bot Listening State
      isBotListening = data.is_active;
      if (isBotListening) {
        statusDot.className = "pulse-dot";
        statusText.textContent = "Listening Active";
        btnToggleBot.className = "btn-toggle-bot";
        btnToggleLabel.textContent = "Pause";
      } else {
        statusDot.className = "pulse-dot paused";
        statusText.textContent = "Paused";
        btnToggleBot.className = "btn-toggle-bot active-start";
        btnToggleLabel.textContent = "Resume";
      }

      // Stats
      if (data.stats) {
        statDetected.textContent = data.stats.detected || 0;
        statClaimed.textContent = data.stats.claimed || 0;
        statFailed.textContent = data.stats.failed || 0;

        const total = (data.stats.claimed || 0) + (data.stats.failed || 0);
        if (total > 0) {
          const rate = Math.round(((data.stats.claimed || 0) / total) * 100);
          statSuccessRate.textContent = `${rate}%`;
        } else {
          statSuccessRate.textContent = "100%";
        }
      }

      // Update Quick Config
      if (data.config) {
        const cmdEl = document.getElementById("summary-cmd");
        const fmtEl = document.getElementById("summary-fmt");
        const iqdbEl = document.getElementById("summary-iqdb");
        const traceEl = document.getElementById("summary-tracemoe");
        const trigEl = document.getElementById("summary-triggers");

        if (cmdEl) cmdEl.textContent = data.config.CLAIM_COMMAND || "/protecc";
        if (fmtEl) fmtEl.textContent = data.config.NAME_FORMAT || "full";
        if (iqdbEl) iqdbEl.textContent = `${data.config.IQDB_MIN_SIMILARITY}%`;
        if (traceEl) {
          const tVal = data.config.TRACEMOE_MIN_SIMILARITY <= 1.0 
            ? Math.round(data.config.TRACEMOE_MIN_SIMILARITY * 100) 
            : Math.round(data.config.TRACEMOE_MIN_SIMILARITY);
          traceEl.textContent = `${tVal}%`;
        }
        if (trigEl) trigEl.textContent = data.config.TRIGGER_KEYWORDS || "--";
      }
    } catch (err) {
      console.warn("Status fetch failed:", err);
    }
  }

  btnToggleBot.addEventListener("click", async () => {
    try {
      btnToggleBot.disabled = true;
      const res = await authedFetch("/api/bot/toggle", { method: "POST" });
      const data = await res.json();
      await fetchStatus();
    } catch (err) {
      alert("Gagal mengubah status bot: " + err.message);
    } finally {
      btnToggleBot.disabled = false;
    }
  });

  // Polling status every 4 seconds
  fetchStatus();
  setInterval(fetchStatus, 4000);

  // ==========================================
  // 2. Real-time Event Stream (SSE)
  // ==========================================
  const feedList = document.getElementById("feed-list");
  const feedEmpty = document.getElementById("feed-empty");
  const feedCount = document.getElementById("feed-count");
  const consoleOutput = document.getElementById("console-output");
  const chkAutoscroll = document.getElementById("chk-autoscroll");
  const btnClearLogs = document.getElementById("btn-clear-logs");
  const selLogFilter = document.getElementById("sel-log-filter");

  let eventsCount = 0;

  function matchesLogFilter(level, filterVal) {
    if (!filterVal || filterVal === "all") return true;
    if (filterVal === "success") return level === "success";
    if (filterVal === "warning") return level === "warning" || level === "error";
    return true;
  }

  function appendLog(text, level = "info") {
    if (!consoleOutput) return;
    const line = document.createElement("div");
    line.className = `log-line ${level}`;
    line.dataset.level = level;
    line.textContent = text;

    const currentFilter = selLogFilter ? selLogFilter.value : "all";
    if (!matchesLogFilter(level, currentFilter)) {
      line.style.display = "none";
    }

    consoleOutput.appendChild(line);

    // Limit log lines to 250 in DOM to prevent browser lag
    if (consoleOutput.children.length > 250) {
      consoleOutput.removeChild(consoleOutput.children[0]);
    }

    if (chkAutoscroll && chkAutoscroll.checked) {
      consoleOutput.scrollTop = consoleOutput.scrollHeight;
    }
  }

  if (selLogFilter) {
    selLogFilter.addEventListener("change", () => {
      const val = selLogFilter.value;
      const lines = consoleOutput.querySelectorAll(".log-line");
      lines.forEach((l) => {
        const lvl = l.dataset.level || "info";
        l.style.display = matchesLogFilter(lvl, val) ? "" : "none";
      });
      if (chkAutoscroll && chkAutoscroll.checked) {
        consoleOutput.scrollTop = consoleOutput.scrollHeight;
      }
    });
  }

  if (btnClearLogs) {
    btnClearLogs.addEventListener("click", () => {
      consoleOutput.innerHTML = '<div class="log-line system" data-level="system">[LOG DIBERSIHKAN]</div>';
    });
  }

  function renderFeedCard(eventData) {
    if (!feedList) return;
    if (feedEmpty) feedEmpty.style.display = "none";

    const item = document.createElement("div");
    item.className = "feed-item";

    const isSuccess = eventData.success;
    const timeStr = eventData.timestamp
      ? new Date(eventData.timestamp * 1000).toLocaleTimeString()
      : new Date().toLocaleTimeString();

    item.innerHTML = `
      <div class="feed-item-left">
        <div class="feed-avatar-pill">🌸</div>
        <div>
          <span class="feed-char-name">${escapeHtml(eventData.character_name || "Karakter Tidak Dikenal")}</span>
          <span class="feed-char-series">${escapeHtml(eventData.series || "Unknown Anime/Game")} • ${eventData.confidence || "--"}% (${eventData.source || "IQDB"})</span>
        </div>
      </div>
      <div class="feed-item-right">
        <span class="claim-tag ${isSuccess ? "success" : "fail"}">
          ${isSuccess ? "✅ BERHASIL: " + escapeHtml(eventData.claim_name || "") : "❌ DITOLAK BOT"}
        </span>
        <span class="feed-time">${timeStr}</span>
      </div>
    `;

    feedList.insertBefore(item, feedList.firstChild);
    eventsCount++;
    if (feedCount) feedCount.textContent = `${eventsCount} items`;

    // Cap feed list items to 40
    if (feedList.children.length > 40) {
      feedList.removeChild(feedList.lastChild);
    }
  }

  // SSE setup
  function connectSSE() {
    const sse = new EventSource("/api/events");

    sse.onopen = () => {
      appendLog("[SISTEM] Terhubung dengan server background.", "success");
    };

    sse.addEventListener("claim_event", (e) => {
      try {
        const data = JSON.parse(e.data);
        renderFeedCard(data);
        fetchStatus();
      } catch (err) {
        console.error("Error parsing claim event:", err);
      }
    });

    sse.addEventListener("log", (e) => {
      try {
        const data = JSON.parse(e.data);
        appendLog(data.message, data.level || "info");
      } catch {
        appendLog(e.data, "info");
      }
    });

    sse.onerror = () => {
      appendLog("[SISTEM] Koneksi terputus. Mencoba reconnect otomatis dalam 3 detik...", "warning");
      sse.close();
      setTimeout(connectSSE, 3000);
    };
  }

  connectSSE();

  // ==========================================
  // 3. Interactive Waifu Tester
  // ==========================================
  const dropzone = document.getElementById("test-dropzone");
  const fileInput = document.getElementById("test-file-input");
  const dropzoneContent = document.getElementById("dropzone-content");
  const dropzonePreview = document.getElementById("dropzone-preview");
  const previewImg = document.getElementById("preview-img");
  const btnClearPreview = document.getElementById("btn-clear-preview");
  const btnBrowseFile = document.getElementById("btn-browse-file");
  const btnRunTest = document.getElementById("btn-run-test");

  const testPlaceholder = document.getElementById("test-placeholder");
  const testResultCard = document.getElementById("test-result-card");
  const testStatusBadge = document.getElementById("test-status-badge");

  const resFullname = document.getElementById("res-fullname");
  const resSeries = document.getElementById("res-series");
  const resFirstname = document.getElementById("res-firstname");
  const resLastname = document.getElementById("res-lastname");
  const resConfidence = document.getElementById("res-confidence");
  const resEngine = document.getElementById("res-engine");
  const resCmdCurrent = document.getElementById("res-cmd-current");
  const resCmdFallbacks = document.getElementById("res-cmd-fallbacks");

  let selectedFile = null;

  btnBrowseFile.addEventListener("click", () => fileInput.click());
  dropzone.addEventListener("click", (e) => {
    if (e.target === dropzone || e.target.closest("#dropzone-content")) {
      fileInput.click();
    }
  });

  fileInput.addEventListener("change", (e) => {
    if (e.target.files && e.target.files[0]) {
      handleSelectedFile(e.target.files[0]);
    }
  });

  // Drag & drop handlers
  ["dragenter", "dragover"].forEach((eventName) => {
    dropzone.addEventListener(eventName, (e) => {
      e.preventDefault();
      dropzone.classList.add("dragover");
    });
  });

  ["dragleave", "drop"].forEach((eventName) => {
    dropzone.addEventListener(eventName, (e) => {
      e.preventDefault();
      dropzone.classList.remove("dragover");
    });
  });

  dropzone.addEventListener("drop", (e) => {
    if (e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files[0]) {
      handleSelectedFile(e.dataTransfer.files[0]);
    }
  });

  function handleSelectedFile(file) {
    if (!file.type.startsWith("image/")) {
      alert("Harap pilih file gambar (JPG, PNG, atau WEBP)!");
      return;
    }
    if (file.size > 8 * 1024 * 1024) {
      alert("Ukuran gambar terlalu besar! Maksimum 8 MB.");
      return;
    }
    selectedFile = file;

    const reader = new FileReader();
    reader.onload = (e) => {
      previewImg.src = e.target.result;
      dropzoneContent.style.display = "none";
      dropzonePreview.style.display = "block";
      btnRunTest.disabled = false;
      testStatusBadge.textContent = "Siap Dianalisis";
      testStatusBadge.className = "badge";
    };
    reader.readAsDataURL(file);
  }

  btnClearPreview.addEventListener("click", (e) => {
    e.stopPropagation();
    selectedFile = null;
    fileInput.value = "";
    previewImg.src = "";
    dropzoneContent.style.display = "flex";
    dropzonePreview.style.display = "none";
    btnRunTest.disabled = true;
    testPlaceholder.style.display = "block";
    testResultCard.style.display = "none";
    testStatusBadge.textContent = "Menunggu Gambar";
    testStatusBadge.className = "badge";
  });

  btnRunTest.addEventListener("click", async () => {
    if (!selectedFile) return;

    btnRunTest.disabled = true;
    btnRunTest.innerHTML = "<span>⏳ Sedang Mencari di Internet...</span>";
    testStatusBadge.textContent = "Menganalisis...";
    testStatusBadge.className = "badge";

    const formData = new FormData();
    formData.append("image", selectedFile);

    try {
      const res = await authedFetch("/api/test-image", {
        method: "POST",
        body: formData,
      });

      const data = await res.json();

      if (!res.ok || !data.success) {
        testPlaceholder.style.display = "block";
        testResultCard.style.display = "none";
        testStatusBadge.textContent = "Tidak Ditemukan";
        testStatusBadge.className = "badge";
        alert(data.message || "Karakter tidak berhasil dikenali di IQDB atau Trace.moe.");
        return;
      }

      // Display result
      const char = data.character;
      resFullname.textContent = char.full_name;
      resSeries.textContent = char.series || "Unknown Anime / Game";
      resFirstname.textContent = char.first_name || "--";
      resLastname.textContent = char.last_name || "--";
      resConfidence.textContent = `${char.confidence}%`;
      resEngine.textContent = char.source || "iqdb_search";

      if (data.commands) {
        resCmdCurrent.textContent = data.commands.primary || "--";
        resCmdFallbacks.textContent = (data.commands.fallbacks || []).join(" ➔ ") || "Tidak ada fallback";
      }

      testPlaceholder.style.display = "none";
      testResultCard.style.display = "block";
      testStatusBadge.textContent = "Dikenali Sukses";
      testStatusBadge.className = "badge";
    } catch (err) {
      alert("Error saat menguji gambar: " + err.message);
    } finally {
      btnRunTest.disabled = false;
      btnRunTest.innerHTML = "<span>🔍 Mulai Identifikasi Gambar</span>";
    }
  });

  // ==========================================
  // ==========================================
  // 4. Settings Manager
  // ==========================================
  const formSettings = document.getElementById("form-settings");
  const saveStatusMsg = document.getElementById("save-status-msg");

  async function loadSettings() {
    try {
      const res = await authedFetch("/api/settings");
      if (!res.ok) return;
      const data = await res.json();

      document.getElementById("cfg-claim-cmd").value = data.CLAIM_COMMAND || "/protecc";
      document.getElementById("cfg-name-fmt").value = data.NAME_FORMAT || "full";
      document.getElementById("cfg-iqdb-sim").value = data.IQDB_MIN_SIMILARITY != null ? data.IQDB_MIN_SIMILARITY : 60.0;
      
      const traceSim = data.TRACEMOE_MIN_SIMILARITY != null ? data.TRACEMOE_MIN_SIMILARITY : 0.85;
      document.getElementById("cfg-trace-sim").value = traceSim <= 1.0 ? Math.round(traceSim * 100) : Math.round(traceSim);

      if (document.getElementById("cfg-saucenao-key")) {
        document.getElementById("cfg-saucenao-key").value = data.SAUCENAO_API_KEY || "";
      }
      if (document.getElementById("cfg-saucenao-sim")) {
        document.getElementById("cfg-saucenao-sim").value = data.SAUCENAO_MIN_SIMILARITY != null ? data.SAUCENAO_MIN_SIMILARITY : 70.0;
      }
      if (document.getElementById("cfg-lens-enabled")) {
        document.getElementById("cfg-lens-enabled").checked = data.LENS_ENABLED !== false;
      }
      document.getElementById("cfg-min-delay").value = data.MIN_DELAY_SECONDS != null ? data.MIN_DELAY_SECONDS : 0.5;
      document.getElementById("cfg-max-delay").value = data.MAX_DELAY_SECONDS != null ? data.MAX_DELAY_SECONDS : 1.5;
      document.getElementById("cfg-triggers").value = data.TRIGGER_KEYWORDS || "";
      document.getElementById("cfg-target-chats").value = data.TARGET_CHAT_IDS || "";
    } catch (err) {
      console.warn("Gagal memuat pengaturan:", err);
    }
  }

  loadSettings();

  formSettings.addEventListener("submit", async (e) => {
    e.preventDefault();
    const btnSave = document.getElementById("btn-save-settings");
    btnSave.disabled = true;
    saveStatusMsg.textContent = "Menyimpan...";
    saveStatusMsg.className = "save-status";

    function parseNum(id, fallback) {
      const el = document.getElementById(id);
      if (!el) return fallback;
      const v = parseFloat(el.value);
      return isNaN(v) ? fallback : v;
    }

    const payload = {
      CLAIM_COMMAND: (document.getElementById("cfg-claim-cmd").value || "/protecc").trim(),
      NAME_FORMAT: document.getElementById("cfg-name-fmt").value || "full",
      IQDB_MIN_SIMILARITY: parseNum("cfg-iqdb-sim", 60.0),
      TRACEMOE_MIN_SIMILARITY: parseNum("cfg-trace-sim", 85.0),
      SAUCENAO_API_KEY: document.getElementById("cfg-saucenao-key") ? document.getElementById("cfg-saucenao-key").value.trim() : "",
      SAUCENAO_MIN_SIMILARITY: parseNum("cfg-saucenao-sim", 70.0),
      LENS_ENABLED: document.getElementById("cfg-lens-enabled") ? document.getElementById("cfg-lens-enabled").checked : true,
      MIN_DELAY_SECONDS: parseNum("cfg-min-delay", 0.5),
      MAX_DELAY_SECONDS: parseNum("cfg-max-delay", 1.5),
      TRIGGER_KEYWORDS: document.getElementById("cfg-triggers").value.trim(),
      TARGET_CHAT_IDS: document.getElementById("cfg-target-chats").value.trim(),
    };

    try {
      const res = await authedFetch("/api/settings", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });

      const data = await res.json();
      if (data.success) {
        saveStatusMsg.textContent = "✅ Konfigurasi berhasil disimpan dan langsung aktif!";
        saveStatusMsg.className = "save-status success";
        fetchStatus();
      } else {
        saveStatusMsg.textContent = "❌ Gagal: " + (data.message || "Terjadi kesalahan");
        saveStatusMsg.className = "save-status error";
      }
    } catch (err) {
      saveStatusMsg.textContent = "❌ Error: " + err.message;
      saveStatusMsg.className = "save-status error";
    } finally {
      btnSave.disabled = false;
      setTimeout(() => {
        saveStatusMsg.textContent = "";
      }, 5000);
    }
  });

  // Utility
  function escapeHtml(str) {
    if (!str) return "";
    return String(str)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#039;");
  }
});
