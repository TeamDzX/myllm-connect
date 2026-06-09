const invoke = window.__TAURI__.core.invoke;

const $ = (id) => document.getElementById(id);
const panels = ["welcome", "idle", "ollama", "working", "paired", "guide"];
const show = (id) => panels.forEach((p) => $(p).classList.toggle("hidden", p !== id));
const setStatus = (cls, text) => {
  $("dot").className = "dot " + cls;
  $("statusText").textContent = text;
};

// True while the user is going through the first-run flow; lets us mark
// onboarding complete once the first pairing QR appears.
let onboarding = false;

// ---- First run: check the local model, then hand off to the share flow ----

async function checkOllama() {
  show("working");
  setStatus("amber", "Checking your model…");
  let st;
  try {
    st = await invoke("ollama_status");
  } catch (e) {
    return ollamaPanel("Couldn’t check your model: " + e, null, null);
  }
  switch (st.state) {
    case "ready":
      return share(); // model present — continue to the secure link + QR
    case "not_installed":
      return ollamaPanel(
        "To run a model on this computer, install Ollama — it’s free and open. Install it, then check again.",
        "Get Ollama", "https://ollama.com/download");
    case "not_running":
      return ollamaPanel(
        "Ollama is installed but not open. Start the Ollama app, then check again.",
        null, null);
    case "no_model":
      return ollamaPanel(
        "Ollama is running, but there’s no model yet. In Terminal, run one of these, then check again:" +
          "<br><br><code>ollama pull llama3.2</code> · 2 GB, fast" +
          "<br><code>ollama pull qwen2.5</code> · 4.7 GB, capable" +
          "<br><code>ollama pull gemma2:2b</code> · 1.6 GB, tiny",
        null, null);
    default:
      return ollamaPanel("Couldn’t check your model.", null, null);
  }
}

function ollamaPanel(html, btnLabel, btnUrl) {
  setStatus("red", "Action needed");
  $("ollamaText").innerHTML = html;
  const b = $("ollamaBtn");
  if (btnLabel) {
    b.textContent = btnLabel;
    b.classList.remove("hidden");
    b.onclick = () => btnUrl && invoke("open_url", { url: btnUrl });
  } else {
    b.classList.add("hidden");
  }
  show("ollama");
}

// ---- Share / secure link / QR (used in first run and steady state) ----

async function share() {
  show("working");
  setStatus("amber", "Setting up…");
  let res;
  try {
    res = await invoke("start_sharing");
  } catch (e) {
    return guide("Something went wrong: " + e, null, null);
  }
  render(res);
}

function render(res) {
  // No model ready? Route to the model step first (covers steady state too).
  if (!res.ollama_up || !res.model) return checkOllama();

  const ts = res.tailscale;
  if (ts.state === "healthy" && res.qr_svg) {
    $("qr").innerHTML = res.qr_svg;
    $("modelLine").textContent = res.model ? "Sharing: " + res.model : "";
    setStatus("green", "Ready to pair");
    show("paired");
    if (onboarding) { // first successful setup — don't show the wizard again
      onboarding = false;
      invoke("set_onboarded");
    }
    return;
  }
  switch (ts.state) {
    case "not_installed":
      return guide(
        "MyLLM Connect needs Tailscale to create a secure link to your phone. Install it once, sign in, then try again.",
        "Get Tailscale", "https://tailscale.com/download");
    case "cli_not_linked":
      return guide(
        "Tailscale is installed, but its command-line tool isn’t enabled yet. Open Tailscale, choose “Install CLI” from its menu bar icon, then try again.",
        "Open Tailscale", "/Applications/Tailscale.app");
    case "logged_out":
      return guide("Almost there — open Tailscale and sign in, then try again.", null, null);
    case "serve_not_enabled":
      return guide(
        "One more step: allow secure connections for your account (a one-time setting), then try again.",
        ts.enable_url ? "Open settings" : null, ts.enable_url);
    default:
      return guide(
        "Couldn’t set up the secure connection" + (ts.message ? ": " + ts.message : "") + ".",
        null, null);
  }
}

function guide(text, btnLabel, btnUrl) {
  setStatus("red", "Action needed");
  $("guideText").textContent = text;
  const b = $("guideBtn");
  if (btnLabel) {
    b.textContent = btnLabel;
    b.classList.remove("hidden");
    b.onclick = () => btnUrl && invoke("open_url", { url: btnUrl });
  } else {
    b.classList.add("hidden");
  }
  show("guide");
}

// ---- Wiring ----

$("startBtn").onclick = checkOllama; // begin first-run setup at the model step
$("skipBtn").onclick = async () => {
  await invoke("set_onboarded");
  onboarding = false;
  setStatus("grey", "Not sharing yet");
  show("idle");
};
$("ollamaRecheck").onclick = checkOllama;
$("shareBtn").onclick = share;
$("retryBtn").onclick = share;
$("rotateBtn").onclick = async () => {
  show("working");
  setStatus("amber", "Rotating…");
  try {
    render(await invoke("rotate_key"));
  } catch (e) {
    guide("Couldn’t rotate the key: " + e, null, null);
  }
};

// First launch shows the wizard; afterwards, the steady-state share view.
(async () => {
  let done = false;
  try { done = await invoke("is_onboarded"); } catch {}
  if (done) {
    setStatus("grey", "Not sharing yet");
    show("idle");
  } else {
    onboarding = true;
    show("welcome");
  }
})();
