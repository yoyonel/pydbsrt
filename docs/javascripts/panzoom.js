/**
 * Interactive Pan, Zoom & Fullscreen Viewer for Mermaid Diagrams
 * Compatible with Material for MkDocs & Light/Dark themes
 */
(() => {
  const initViewer = (originalEl) => {
    if (originalEl.dataset.mermaidViewerInit) return;

    // Look for SVG in light DOM or open shadow DOM
    let svg = originalEl.querySelector("svg");
    if (!svg && originalEl.shadowRoot) {
      svg = originalEl.shadowRoot.querySelector("svg");
    }
    if (!svg) return;

    originalEl.dataset.mermaidViewerInit = "true";

    // Clone SVG to place it into Light DOM
    const clonedSvg = svg.cloneNode(true);
    clonedSvg.removeAttribute("id");
    clonedSvg.style.maxWidth = "none";
    clonedSvg.style.width = "100%";
    clonedSvg.style.height = "100%";
    clonedSvg.style.display = "block";
    clonedSvg.style.userSelect = "none";
    clonedSvg.style.pointerEvents = "all";

    // Wrap SVG children into a single zoom layer group
    const zoomGroup = document.createElementNS("http://www.w3.org/2000/svg", "g");
    zoomGroup.setAttribute("class", "mermaid-zoom-layer");
    while (clonedSvg.firstChild) {
      zoomGroup.appendChild(clonedSvg.firstChild);
    }
    clonedSvg.appendChild(zoomGroup);

    // Build the Card UI
    const card = document.createElement("div");
    card.className = "mermaid-card";

    // Header Toolbar
    const header = document.createElement("div");
    header.className = "mermaid-header";
    header.innerHTML = `
      <div class="mermaid-badge">
        <svg class="mermaid-icon" viewBox="0 0 24 24" width="16" height="16">
          <path fill="currentColor" d="M3 3h8v8H3V3m10 0h8v8h-8V3M3 13h8v8H3v-8m10 0h8v8h-8v-8Z"/>
        </svg>
        <span>Diagramme Interactif</span>
      </div>
      <div class="mermaid-hint">Molette : Zoom • Glisser : Naviguer • Double-clic : Plein Écran</div>
      <div class="mermaid-toolbar">
        <button type="button" class="mermaid-btn btn-zoom-in" title="Zoomer (+)">
          <svg viewBox="0 0 24 24" width="16" height="16"><path fill="currentColor" d="M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2Z"/></svg>
        </button>
        <button type="button" class="mermaid-btn btn-zoom-out" title="Dézoomer (−)">
          <svg viewBox="0 0 24 24" width="16" height="16"><path fill="currentColor" d="M19 13H5v-2h14v2Z"/></svg>
        </button>
        <button type="button" class="mermaid-btn btn-reset" title="Réinitialiser (100%)">
          <svg viewBox="0 0 24 24" width="16" height="16"><path fill="currentColor" d="M17.65 6.35A7.958 7.958 0 0 0 12 4c-4.42 0-7.99 3.58-7.99 8s3.57 8 7.99 8c3.73 0 6.84-2.55 7.73-6h-2.08A5.99 5.99 0 0 1 12 18c-3.31 0-6-2.69-6-6s2.69-6 6-6c1.66 0 3.14.69 4.22 1.78L13 11h7V4l-2.35 2.35Z"/></svg>
        </button>
        <button type="button" class="mermaid-btn btn-fullscreen" title="Plein Écran / Modal">
          <svg class="icon-expand" viewBox="0 0 24 24" width="16" height="16"><path fill="currentColor" d="M5 5h5V3H3v7h2V5m14 0h-5V3h7v7h-2V5M5 19h5v2H3v-7h2v5m14 0h-5v2h7v-7h-2v5Z"/></svg>
          <svg class="icon-contract" viewBox="0 0 24 24" width="16" height="16" style="display:none;"><path fill="currentColor" d="M10 4H8v4H4v2h6V4m4 0h-2v6h6V8h-4V4M4 14v2h4v4h2v-6H4m10 0v6h2v-4h4v-2h-6Z"/></svg>
        </button>
      </div>
    `;

    // Viewport
    const viewport = document.createElement("div");
    viewport.className = "mermaid-viewport";
    viewport.appendChild(clonedSvg);

    card.appendChild(header);
    card.appendChild(viewport);

    // Hide original Material container
    originalEl.style.display = "none";
    originalEl.parentNode.insertBefore(card, originalEl);

    // State
    let scale = 1;
    let tx = 0;
    let ty = 0;
    let isDragging = false;
    let startX = 0;
    let startY = 0;
    let hasMoved = false;

    const updateTransform = () => {
      zoomGroup.setAttribute("transform", `translate(${tx}, ${ty}) scale(${scale})`);
    };

    const zoomAt = (factor, clientX, clientY) => {
      const rect = viewport.getBoundingClientRect();
      const x = clientX !== undefined ? clientX - rect.left : rect.width / 2;
      const y = clientY !== undefined ? clientY - rect.top : rect.height / 2;

      const newScale = Math.min(Math.max(0.2, scale * factor), 20);
      tx = x - (x - tx) * (newScale / scale);
      ty = y - (y - ty) * (newScale / scale);
      scale = newScale;
      updateTransform();
    };

    const resetTransform = () => {
      scale = 1;
      tx = 0;
      ty = 0;
      updateTransform();
    };

    // Wheel Zoom
    viewport.addEventListener(
      "wheel",
      (e) => {
        e.preventDefault();
        const factor = e.deltaY < 0 ? 1.15 : 0.87;
        zoomAt(factor, e.clientX, e.clientY);
      },
      { passive: false }
    );

    // Mouse Pan
    viewport.addEventListener("mousedown", (e) => {
      // Ignore if clicking on a button or link
      if (e.target.closest("button") || e.target.closest("a")) return;
      isDragging = true;
      hasMoved = false;
      startX = e.clientX - tx;
      startY = e.clientY - ty;
      viewport.style.cursor = "grabbing";
    });

    window.addEventListener("mousemove", (e) => {
      if (!isDragging) return;
      hasMoved = true;
      tx = e.clientX - startX;
      ty = e.clientY - startY;
      updateTransform();
    });

    window.addEventListener("mouseup", () => {
      if (isDragging) {
        isDragging = false;
        viewport.style.cursor = "grab";
      }
    });

    // Touch Support (Single finger drag & 2-finger pinch)
    let lastTouchDist = null;
    viewport.addEventListener(
      "touchstart",
      (e) => {
        if (e.touches.length === 1) {
          isDragging = true;
          startX = e.touches[0].clientX - tx;
          startY = e.touches[0].clientY - ty;
        } else if (e.touches.length === 2) {
          isDragging = false;
          lastTouchDist = Math.hypot(
            e.touches[0].clientX - e.touches[1].clientX,
            e.touches[0].clientY - e.touches[1].clientY
          );
        }
      },
      { passive: true }
    );

    viewport.addEventListener(
      "touchmove",
      (e) => {
        if (isDragging && e.touches.length === 1) {
          e.preventDefault();
          tx = e.touches[0].clientX - startX;
          ty = e.touches[0].clientY - startY;
          updateTransform();
        } else if (e.touches.length === 2 && lastTouchDist) {
          e.preventDefault();
          const dist = Math.hypot(
            e.touches[0].clientX - e.touches[1].clientX,
            e.touches[0].clientY - e.touches[1].clientY
          );
          const midX = (e.touches[0].clientX + e.touches[1].clientX) / 2;
          const midY = (e.touches[0].clientY + e.touches[1].clientY) / 2;
          zoomAt(dist / lastTouchDist, midX, midY);
          lastTouchDist = dist;
        }
      },
      { passive: false }
    );

    viewport.addEventListener("touchend", () => {
      isDragging = false;
      lastTouchDist = null;
    });

    // Double-click to toggle fullscreen
    viewport.addEventListener("dblclick", (e) => {
      if (hasMoved) return;
      toggleFullscreen();
    });

    // Buttons
    card.querySelector(".btn-zoom-in").addEventListener("click", () => zoomAt(1.3));
    card.querySelector(".btn-zoom-out").addEventListener("click", () => zoomAt(0.77));
    card.querySelector(".btn-reset").addEventListener("click", resetTransform);

    const toggleFullscreen = () => {
      const isFs = card.classList.toggle("is-fullscreen");
      document.body.classList.toggle("has-mermaid-modal", isFs);
      const iconExpand = card.querySelector(".icon-expand");
      const iconContract = card.querySelector(".icon-contract");
      if (iconExpand && iconContract) {
        iconExpand.style.display = isFs ? "none" : "block";
        iconContract.style.display = isFs ? "block" : "none";
      }
      resetTransform();
    };

    card.querySelector(".btn-fullscreen").addEventListener("click", toggleFullscreen);

    // Escape key closes fullscreen
    window.addEventListener("keydown", (e) => {
      if (e.key === "Escape" && card.classList.contains("is-fullscreen")) {
        toggleFullscreen();
      }
    });
  };

  const scanAndInit = () => {
    document.querySelectorAll(".mermaid").forEach((el) => {
      initViewer(el);
    });
  };

  // Trigger on Material navigation and timer intervals
  if (window.document$) {
    window.document$.subscribe(() => {
      scanAndInit();
      [150, 400, 800, 1500, 3000].forEach((ms) => setTimeout(scanAndInit, ms));
    });
  } else {
    document.addEventListener("DOMContentLoaded", () => {
      scanAndInit();
      [150, 400, 800, 1500, 3000].forEach((ms) => setTimeout(scanAndInit, ms));
    });
  }

  // MutationObserver for any dynamic insertions
  const obs = new MutationObserver(scanAndInit);
  obs.observe(document.documentElement, { childList: true, subtree: true });
})();
