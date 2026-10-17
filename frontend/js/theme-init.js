// Runs before first paint (loaded synchronously in <head>) so the saved theme
// is applied without a flash of the wrong colours.
(function () {
  var theme = 'light';
  try {
    theme = localStorage.getItem('vm.theme') ||
      (window.matchMedia && window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light');
  } catch (e) { /* storage blocked: keep default */ }
  document.documentElement.setAttribute('data-theme', theme);
})();
