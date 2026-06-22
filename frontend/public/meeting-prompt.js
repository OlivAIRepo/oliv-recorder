(function () {
  var T = window.__TAURI__;
  if (!T) return;
  var listen = T.event.listen;
  var emit = T.event.emit;
  var invoke = T.core.invoke;
  var getCurrentWindow = T.window.getCurrentWindow;

  var titleEl = document.getElementById("title");
  var subEl = document.getElementById("sub");
  var sensitiveRow = document.getElementById("sensitiveRow");
  var sensitiveEl = document.getElementById("sensitive");
  var btnPrimary = document.getElementById("btnPrimary");
  var btnSecondary = document.getElementById("btnSecondary");

  var COUNTDOWN = 10;
  var mode = "detect"; // "detect" | "ended"
  var currentApp = "a meeting app";
  var seconds = COUNTDOWN;
  var timer = null;
  var done = false;

  function stopTimer() {
    if (timer) { clearInterval(timer); timer = null; }
  }

  // Close via Rust so a hidden main window / app isn't surfaced when we hide.
  function close() {
    invoke("close_meeting_prompt").catch(function () {
      try { getCurrentWindow().hide(); } catch (e) {}
    });
  }

  function renderSub() {
    if (mode === "detect") {
      subEl.innerHTML = "<b></b> · starts in <b>" + seconds + "s</b>";
      subEl.firstChild.textContent = currentApp;
    } else {
      subEl.innerHTML = "Ending in <b>" + seconds + "s</b>";
    }
  }

  function tick() {
    seconds -= 1;
    if (seconds <= 0) {
      stopTimer();
      doPrimary(); // auto-fire: detect → Start, ended → End
    } else {
      renderSub();
    }
  }

  function startCountdown() {
    seconds = COUNTDOWN;
    done = false;
    stopTimer();
    renderSub();
    timer = setInterval(tick, 1000);
  }

  // Primary (right) — the auto-fired default action.
  function doPrimary() {
    if (done) return;
    done = true;
    stopTimer();
    if (mode === "detect") {
      emit("start-recording-from-prompt", { app: currentApp, sensitive: sensitiveEl.checked });
    } else {
      // "End" → stop transcription in the background (non-focusing stop).
      invoke("oliv_stop_recording").catch(function () {});
    }
    close(); // hides the app so it never comes to the foreground
  }

  // Secondary (left) — detect → Dismiss, ended → Continue (keep transcribing).
  function doSecondary() {
    if (done) return;
    done = true;
    stopTimer();
    close();
  }

  // --- Meeting detected: 10s → auto Start. Mic-only toggle shown. ---
  function renderDetect(app) {
    mode = "detect";
    currentApp = app || "a meeting app";
    titleEl.textContent = "Meeting detected";
    sensitiveRow.classList.remove("hidden");
    sensitiveEl.checked = false;
    btnSecondary.textContent = "Dismiss";
    btnSecondary.className = "secondary";
    btnPrimary.textContent = "Start transcription";
    btnPrimary.className = "primary";
    startCountdown();
  }

  // --- Meeting ended: 10s → auto End. End is primary (right), Continue left. ---
  function renderEnded() {
    mode = "ended";
    titleEl.textContent = "Meeting ended";
    sensitiveRow.classList.add("hidden");
    btnSecondary.textContent = "Continue";
    btnSecondary.className = "secondary";
    btnPrimary.textContent = "End";
    btnPrimary.className = "danger";
    startCountdown();
  }

  btnPrimary.addEventListener("click", doPrimary);
  btnSecondary.addEventListener("click", doSecondary);

  listen("meeting-detected", function (e) {
    renderDetect(((e && e.payload) || {}).app);
  });
  listen("meeting-ended", function () {
    renderEnded();
  });
  // A new call started while transcribing → cancel the pending auto-end.
  listen("meeting-resumed", function () {
    if (mode === "ended") {
      stopTimer();
      close();
    }
  });
})();
