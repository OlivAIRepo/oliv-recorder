(function () {
  var T = window.__TAURI__;
  if (!T) return;
  var listen = T.event.listen;
  var emit = T.event.emit;
  var getCurrentWindow = T.window.getCurrentWindow;

  var COUNTDOWN = 20;
  var appEl = document.getElementById("app");
  var secsEl = document.getElementById("secs");
  var sensitiveEl = document.getElementById("sensitive");

  var currentApp = "a meeting app";
  var seconds = COUNTDOWN;
  var timer = null;
  var started = false;

  function hide() {
    try { getCurrentWindow().hide(); } catch (e) {}
  }
  function stopTimer() {
    if (timer) { clearInterval(timer); timer = null; }
  }
  function start() {
    if (started) return;
    started = true;
    stopTimer();
    emit("start-recording-from-prompt", { app: currentApp, sensitive: sensitiveEl.checked });
    hide();
  }
  function dismiss() {
    started = true;
    stopTimer();
    hide();
  }
  function tick() {
    seconds -= 1;
    secsEl.textContent = seconds + "s";
    if (seconds <= 0) start();
  }
  function onDetected(app) {
    started = false;
    currentApp = app || "a meeting app";
    appEl.textContent = currentApp;
    sensitiveEl.checked = false;
    seconds = COUNTDOWN;
    secsEl.textContent = seconds + "s";
    stopTimer();
    timer = setInterval(tick, 1000);
  }

  document.getElementById("start").addEventListener("click", start);
  document.getElementById("dismiss").addEventListener("click", dismiss);

  listen("meeting-detected", function (e) {
    var p = e && e.payload ? e.payload : {};
    onDetected(p.app);
  });
  // Meeting vanished before the user acted — close the prompt.
  listen("meeting-ended", function () {
    dismiss();
  });
})();
