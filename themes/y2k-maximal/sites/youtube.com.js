/* Seu Navegador — Y2K Maximal YouTube Skin (Isolated JS, SPA-aware) */
(function () {
  var PREFS_KEY = "seu-y2k-yt-prefs-v1";
  var BAR_ID = "seu-yt-y2k-bar";
  var observer = null;
  var scanTimer = null;

  function loadPrefs() {
    try {
      var raw = localStorage.getItem(PREFS_KEY);
      if (raw) {
        var parsed = JSON.parse(raw);
        return {
          hideShorts: parsed.hideShorts !== false,
          widePlayer: parsed.widePlayer === true
        };
      }
    } catch (_) {}
    return { hideShorts: true, widePlayer: false };
  }

  function savePrefs(prefs) {
    try {
      localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
    } catch (_) {}
  }

  var prefs = loadPrefs();

  function syncAttributes() {
    var root = document.documentElement;
    if (!root) return;
    root.setAttribute("data-seu-yt-skin", "y2k-maximal");
    root.setAttribute("data-seu-yt-hide-shorts", prefs.hideShorts ? "true" : "false");
    root.setAttribute("data-seu-yt-wide", prefs.widePlayer ? "true" : "false");
  }

  function scanShortsElements() {
    var links = document.querySelectorAll('a[href^="/shorts/"], a[href*="/shorts/"]');
    for (var i = 0; i < links.length; i++) {
      var card = links[i].closest(
        "ytd-rich-item-renderer, ytd-video-renderer, ytd-reel-item-renderer, ytm-shorts-lockup-view-model"
      );
      if (card) {
        if (prefs.hideShorts) {
          card.setAttribute("data-seu-yt-shorts-hidden", "true");
        } else {
          card.removeAttribute("data-seu-yt-shorts-hidden");
        }
      }
    }
  }

  function scheduleScan() {
    if (scanTimer) return;
    scanTimer = setTimeout(function () {
      scanTimer = null;
      syncAttributes();
      scanShortsElements();
      ensureControlBar();
    }, 140);
  }

  function ensureControlBar() {
    if (!document.body) return;
    var bar = document.getElementById(BAR_ID);
    if (!bar) {
      bar = document.createElement("div");
      bar.id = BAR_ID;
      bar.setAttribute("role", "toolbar");
      bar.setAttribute("aria-label", "Y2K YouTube Skin");

      var tag = document.createElement("span");
      tag.className = "seu-yt-tag";
      tag.textContent = "Y2K";
      bar.appendChild(tag);

      var shortsBtn = document.createElement("button");
      shortsBtn.type = "button";
      shortsBtn.dataset.seuControl = "shorts";
      shortsBtn.addEventListener("click", function () {
        prefs.hideShorts = !prefs.hideShorts;
        savePrefs(prefs);
        syncAttributes();
        scanShortsElements();
        updateBarButtons(bar);
      });
      bar.appendChild(shortsBtn);

      var wideBtn = document.createElement("button");
      wideBtn.type = "button";
      wideBtn.dataset.seuControl = "wide";
      wideBtn.addEventListener("click", function () {
        prefs.widePlayer = !prefs.widePlayer;
        savePrefs(prefs);
        syncAttributes();
        updateBarButtons(bar);
      });
      bar.appendChild(wideBtn);

      document.body.appendChild(bar);
    }
    updateBarButtons(bar);
  }

  function updateBarButtons(bar) {
    var shortsBtn = bar.querySelector('button[data-seu-control="shorts"]');
    if (shortsBtn) {
      shortsBtn.setAttribute("aria-pressed", prefs.hideShorts ? "true" : "false");
      shortsBtn.textContent = prefs.hideShorts ? "Sem Shorts: ON" : "Sem Shorts: OFF";
    }
    var wideBtn = bar.querySelector('button[data-seu-control="wide"]');
    if (wideBtn) {
      wideBtn.setAttribute("aria-pressed", prefs.widePlayer ? "true" : "false");
      wideBtn.textContent = prefs.widePlayer ? "Player Amplo: ON" : "Player Amplo";
    }
  }

  function onNavigate() {
    syncAttributes();
    scanShortsElements();
    ensureControlBar();
  }

  function teardown() {
    if (observer) {
      observer.disconnect();
      observer = null;
    }
    if (scanTimer) {
      clearTimeout(scanTimer);
      scanTimer = null;
    }
    var bar = document.getElementById(BAR_ID);
    if (bar && bar.parentNode) bar.parentNode.removeChild(bar);
    var root = document.documentElement;
    if (root) {
      root.removeAttribute("data-seu-yt-skin");
      root.removeAttribute("data-seu-yt-hide-shorts");
      root.removeAttribute("data-seu-yt-wide");
    }
    var hidden = document.querySelectorAll("[data-seu-yt-shorts-hidden]");
    for (var i = 0; i < hidden.length; i++) {
      hidden[i].removeAttribute("data-seu-yt-shorts-hidden");
    }
    window.removeEventListener("yt-navigate-finish", onNavigate);
    window.removeEventListener("seu:skin-navigate", onNavigate);
    window.removeEventListener("seu:skin-disable", teardown);
  }

  syncAttributes();
  scanShortsElements();
  ensureControlBar();

  window.addEventListener("yt-navigate-finish", onNavigate);
  window.addEventListener("seu:skin-navigate", onNavigate);
  window.addEventListener("seu:skin-disable", teardown);

  if (typeof MutationObserver !== "undefined" && document.documentElement) {
    observer = new MutationObserver(scheduleScan);
    observer.observe(document.documentElement, { childList: true, subtree: true });
  }
})();
