window.MathJax = {
  tex: {
    inlineMath: [["\\(", "\\)"], ["$", "$"]],
    displayMath: [["\\[", "\\]"], ["$$", "$$"]],
    processEscapes: true,
    processEnvironments: true
  },
  options: {
    ignoreHtmlClass: ".*|",
    processHtmlClass: "arithmatex"
  }
};

document$.subscribe(() => {
  if (typeof MathJax !== "undefined" && MathJax.typesetPromise) {
    if (MathJax.startup && MathJax.startup.output && typeof MathJax.startup.output.clearCache === "function") {
      MathJax.startup.output.clearCache();
    }
    if (typeof MathJax.typesetClear === "function") {
      MathJax.typesetClear();
    }
    if (typeof MathJax.texReset === "function") {
      MathJax.texReset();
    }
    MathJax.typesetPromise();
  }
});
