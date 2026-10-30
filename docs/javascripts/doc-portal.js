/**
 * dbsrt Documentation Portal Interactive Enhancements
 * - Instant Catalogue Filter Pills & Live Search on Home Dashboard
 * - Automatic Article Metadata Header formatting & Reading Time badge
 * - Environment-aware Local Stack Detection (hidden on static GitHub Pages)
 */
(function () {
  function isLocalEnvironment() {
    const h = window.location.hostname;
    return h === "127.0.0.1" || h === "localhost" || h === "0.0.0.0";
  }

  function initDocPortal() {
    initCatalogFilters();
    enhanceArticleMetadataBanner();
    probeLocalServicesIfLocalhost();
    initAsciinemaPlayers();
    initSearchResultsEnhancer();
  }

  function getDocsDates() {
    return window.__DOCS_DATES__ || {};
  }

  function normalizeDocPath(href) {
    if (!href) return "";
    let clean = href.split("?")[0].split("#")[0];
    clean = clean.replace(/^(?:https?:\/\/[^\/]+)/, "");
    clean = clean.replace(/^(?:\/?\.\.\/)+/, "").replace(/^\/+/, "");
    clean = clean.replace(/^pydbsrt\//, "");
    if (clean && !clean.endsWith("/")) clean += "/";
    return clean;
  }

  function initSearchResultsEnhancer() {
    const search = document.querySelector(".md-search");
    if (!search || search.hasAttribute("data-date-observer")) return;
    search.setAttribute("data-date-observer", "true");

    const observer = new MutationObserver(() => {
      const dates = getDocsDates();
      const links = search.querySelectorAll(".md-search-result__link:not([data-date-enhanced])");
      links.forEach((link) => {
        link.setAttribute("data-date-enhanced", "true");
        const href = link.getAttribute("href");
        const docKey = normalizeDocPath(href);
        const info = dates[docKey] || (docKey === "index/" ? dates[""] : null);
        if (!info) return;

        const articleEl = link.querySelector("article");
        if (!articleEl || articleEl.querySelector(".dbsrt-search-timestamp")) return;

        const heading = articleEl.querySelector("h1, h2");
        if (!heading) return;

        const timeRow = document.createElement("div");
        timeRow.className = "dbsrt-search-timestamp";
        let badgesHtml =
          '<span class="dbsrt-time-badge created" title="Date de rédaction initiale">📅 Rédigé le ' +
          info.created +
          "</span>";
        if (info.updated && info.updated !== info.created) {
          badgesHtml +=
            '<span class="dbsrt-time-badge updated" title="Dernière mise à jour">🔄 Mis à jour le ' +
            info.updated +
            "</span>";
        }
        timeRow.innerHTML = badgesHtml;
        heading.insertAdjacentElement("afterend", timeRow);
      });
    });

    observer.observe(search, { childList: true, subtree: true });
  }

  function initAsciinemaPlayers() {
    if (typeof AsciinemaPlayer === "undefined" || !AsciinemaPlayer.create) {
      return;
    }
    const elements = document.querySelectorAll(".asciinema-cast:not([data-initialized])");
    elements.forEach((el) => {
      el.setAttribute("data-initialized", "true");
      const castUrl = el.getAttribute("data-cast");
      if (!castUrl) return;

      const rows = parseInt(el.getAttribute("data-rows") || "22", 10);
      const cols = parseInt(el.getAttribute("data-cols") || "92", 10);
      const speed = parseFloat(el.getAttribute("data-speed") || "1.0");
      const autoPlay = el.getAttribute("data-autoplay") !== "false";
      const loop = el.getAttribute("data-loop") !== "false";

      try {
        AsciinemaPlayer.create(castUrl, el, {
          rows: rows,
          cols: cols,
          autoPlay: autoPlay,
          loop: loop,
          speed: speed,
          theme: "nord",
          terminalFontFamily: "'JetBrains Mono', 'Fira Code', Monaco, Menlo, Consolas, monospace",
          terminalFontSize: "13px",
          terminalLineHeight: 1.3,
          fit: "width",
          preload: true,
        });
      } catch (err) {
        console.error("Failed to mount Asciinema player on", el, err);
      }
    });
  }

  function initCatalogFilters() {
    const pillsContainer = document.getElementById("dbsrt-filter-pills");
    const searchInput = document.getElementById("dbsrt-catalog-search");
    if (!pillsContainer && !searchInput) return;

    const toolbar = document.querySelector(".dbsrt-catalog-toolbar");
    if (!toolbar) return;

    let tableEl = toolbar.nextElementSibling;
    while (tableEl && tableEl.tagName !== "TABLE" && !tableEl.querySelector("table")) {
      tableEl = tableEl.nextElementSibling;
    }
    const table = tableEl && tableEl.tagName === "TABLE" ? tableEl : tableEl ? tableEl.querySelector("table") : null;
    if (!table) return;

    const rows = Array.from(table.querySelectorAll("tbody tr"));
    let activeFilter = "all";

    function applyFilter() {
      const query = (searchInput ? searchInput.value : "").trim().toLowerCase();
      rows.forEach((row) => {
        const text = row.textContent.toLowerCase();
        const matchesFilter = activeFilter === "all" || text.includes(activeFilter);
        const matchesSearch = !query || text.includes(query);
        row.style.display = matchesFilter && matchesSearch ? "" : "none";
      });
    }

    if (pillsContainer) {
      const buttons = pillsContainer.querySelectorAll(".dbsrt-filter-btn");
      const allBtn = pillsContainer.querySelector('[data-filter="all"]');
      if (allBtn) {
        allBtn.textContent = "Tous (" + rows.length + ")";
      }
      buttons.forEach((btn) => {
        btn.addEventListener("click", function () {
          buttons.forEach((b) => b.classList.remove("active"));
          btn.classList.add("active");
          activeFilter = btn.getAttribute("data-filter") || "all";
          applyFilter();
        });
      });
    }

    if (searchInput) {
      searchInput.addEventListener("input", applyFilter);
    }
  }

  function enhanceArticleMetadataBanner() {
    const article = document.querySelector(".md-content__inner");
    if (!article || document.querySelector(".dbsrt-hero")) return;
    if (article.querySelector(".dbsrt-reading-bar")) return;

    const h1 = article.querySelector("h1");
    if (!h1) return;

    const words = (article.textContent || "").trim().split(/\s+/).length;
    const readMin = Math.max(1, Math.round(words / 220));

    const badgeBar = document.createElement("div");
    badgeBar.className = "dbsrt-reading-bar";

    const docKey = normalizeDocPath(window.location.pathname);
    const info = getDocsDates()[docKey] || (docKey === "index/" ? getDocsDates()[""] : null);
    let datesHtml = "";
    if (info) {
      datesHtml = '<span class="dbsrt-meta-chip">📅 Rédigé le ' + info.created + "</span>";
      if (info.updated && info.updated !== info.created) {
        datesHtml +=
          '<span class="dbsrt-meta-chip dbsrt-meta-chip-updated">🔄 Mis à jour le ' + info.updated + "</span>";
      }
    }
    badgeBar.innerHTML =
      '<span class="dbsrt-meta-chip">⏱️ Lecture ~' +
      readMin +
      " min (" +
      words +
      ' mots)</span>' +
      datesHtml +
      '<span id="dbsrt-article-local-slot" style="display:inline-flex;gap:0.5rem;"></span>';

    const firstBq = h1.nextElementSibling;
    if (firstBq && firstBq.tagName === "BLOCKQUOTE") {
      article.insertBefore(badgeBar, firstBq);
    } else {
      h1.insertAdjacentElement("afterend", badgeBar);
    }
  }

  function probeLocalServicesIfLocalhost() {
    // Never show localhost links on GitHub Pages or remote static hosts
    if (!isLocalEnvironment()) return;

    const heroSlot = document.getElementById("dbsrt-local-services-slot");
    const articleSlot = document.getElementById("dbsrt-article-local-slot");
    if (!heroSlot && !articleSlot) return;

    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 700);

    fetch("http://127.0.0.1:8085/v1/health", { signal: controller.signal, mode: "cors" })
      .then((res) => {
        clearTimeout(timer);
        if (!res.ok) throw new Error("not ok");
        if (heroSlot) {
          heroSlot.innerHTML =
            '<a href="http://127.0.0.1:8085/studio" target="_blank" class="dbsrt-btn dbsrt-btn-live">🟢 Studio Local (:8085)</a>' +
            '<a href="http://127.0.0.1:8086" target="_blank" class="dbsrt-btn dbsrt-btn-live">🐘 PgWeb (:8086)</a>';
        }
        if (articleSlot) {
          articleSlot.innerHTML =
            '<span class="dbsrt-meta-chip dbsrt-meta-chip-live">🟢 <a href="http://127.0.0.1:8085/studio" target="_blank">Studio :8085</a></span>' +
            '<span class="dbsrt-meta-chip dbsrt-meta-chip-live">🐘 <a href="http://127.0.0.1:8086" target="_blank">PgWeb :8086</a></span>';
        }
      })
      .catch(() => {
        clearTimeout(timer);
        // Daemon is offline locally: show a subtle hint on home hero only
        if (heroSlot) {
          heroSlot.innerHTML =
            '<span class="dbsrt-btn dbsrt-btn-live" style="opacity:0.65;cursor:default;" title="Lancez task stack:up dans votre terminal">⚪ Stack locale arrêtée (task stack:up)</span>';
        }
      });
  }

  if (typeof document$ !== "undefined") {
    document$.subscribe(initDocPortal);
  } else {
    document.addEventListener("DOMContentLoaded", initDocPortal);
  }
})();
