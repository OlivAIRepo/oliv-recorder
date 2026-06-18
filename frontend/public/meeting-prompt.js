(function () {
  var T = window.__TAURI__;
  if (!T) return;
  var listen = T.event.listen;
  var emit = T.event.emit;
  var invoke = T.core.invoke;
  var getCurrentWindow = T.window.getCurrentWindow;

  var titleEl = document.getElementById("title");
  var subEl = document.getElementById("sub");
  var appEl = document.getElementById("app");
  var sensitiveRow = document.getElementById("sensitiveRow");
  var sensitiveEl = document.getElementById("sensitive");
  var btnPrimary = document.getElementById("btnPrimary");
  var btnSecondary = document.getElementById("btnSecondary");

  var mode = "detect"; // "detect" | "ended"
  var currentApp = "a meeting app";

  function hide() {
    try { getCurrentWindow().hide(); } catch (e) {}
  }

  // --- Meeting detected: opt-in start (no countdown). ---
  function renderDetect(app) {
    mode = "detect";
    currentApp = app || "a meeting app";
    titleEl.textContent = "Meeting detected";
    subEl.innerHTML = '<b id="app"></b>';
    document.getElementById("app").textContent = currentApp;
    sensitiveRow.classList.remove("hidden");
    sensitiveEl.checked = false;
    btnSecondary.textContent = "Dismiss";
    btnSecondary.className = "secondary";
    btnPrimary.textContent = "Start transcription";
    btnPrimary.className = "primary";
  }

  // --- Meeting ended: persistent continue/end. ---
  function renderEnded() {
    mode = "ended";
    titleEl.textContent = "Meeting ended";
    subEl.innerHTML = "Keep transcribing this session?";
    sensitiveRow.classList.add("hidden");
    btnSecondary.textContent = "End";
    btnSecondary.className = "danger";
    btnPrimary.textContent = "Continue";
    btnPrimary.className = "primary";
  }

  btnPrimary.addEventListener("click", function () {
    if (mode === "detect") {
      emit("start-recording-from-prompt", { app: currentApp, sensitive: sensitiveEl.checked });
    }
    // ended → "Continue": just keep transcribing.
    hide();
  });

  btnSecondary.addEventListener("click", function () {
    if (mode === "ended") {
      // "End" → stop transcription.
      invoke("oliv_stop_recording").catch(function () {});
    }
    // detect → "Dismiss": close only, never open the app.
    hide();
  });

  listen("meeting-detected", function (e) {
    var p = (e && e.payload) || {};
    renderDetect(p.app);
  });
  listen("meeting-ended", function () {
    renderEnded();
  });
})();
