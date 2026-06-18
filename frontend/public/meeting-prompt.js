(function () {
  var T = window.__TAURI__;
  if (!T) return;
  var listen = T.event.listen;
  var emit = T.event.emit;
  var invoke = T.core.invoke;
  var getCurrentWindow = T.window.getCurrentWindow;

  var titleEl = document.getElementById("title");
  var subEl = document.getElementById("sub");
  var micIcon = document.getElementById("micIcon");
  var sensitiveRow = document.getElementById("sensitiveRow");
  var sensitiveEl = document.getElementById("sensitive");
  var btnPrimary = document.getElementById("btnPrimary");
  var btnSecondary = document.getElementById("btnSecondary");

  var mode = "detect"; // "detect" | "ended"
  var currentApp = "a meeting app";

  // Close via Rust so a hidden main window isn't surfaced when the prompt hides.
  function close() {
    invoke("close_meeting_prompt").catch(function () {
      try { getCurrentWindow().hide(); } catch (e) {}
    });
  }

  // --- Meeting detected: opt-in start (no countdown). Mic icon shown. ---
  function renderDetect(app) {
    mode = "detect";
    currentApp = app || "a meeting app";
    titleEl.textContent = "Meeting detected";
    subEl.innerHTML = '<b id="app"></b>';
    document.getElementById("app").textContent = currentApp;
    micIcon.classList.remove("hidden");
    sensitiveRow.classList.remove("hidden");
    sensitiveEl.checked = false;
    btnSecondary.textContent = "Dismiss";
    btnSecondary.className = "secondary";
    btnPrimary.textContent = "Start transcription";
    btnPrimary.className = "primary";
  }

  // --- Meeting ended: persistent continue/end. No mic icon. ---
  function renderEnded() {
    mode = "ended";
    titleEl.textContent = "Meeting ended";
    subEl.innerHTML = "Keep transcribing this session?";
    micIcon.classList.add("hidden");
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
    close();
  });

  btnSecondary.addEventListener("click", function () {
    if (mode === "ended") {
      // "End" → stop transcription (the stop path surfaces the app to show
      // progress); just hide this prompt.
      invoke("oliv_stop_recording").catch(function () {});
      try { getCurrentWindow().hide(); } catch (e) {}
    } else {
      // "Dismiss" → close only, never open the app.
      close();
    }
  });

  listen("meeting-detected", function (e) {
    var p = (e && e.payload) || {};
    renderDetect(p.app);
  });
  listen("meeting-ended", function () {
    renderEnded();
  });
})();
