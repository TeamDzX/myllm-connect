const invoke = window.__TAURI__.core.invoke;

const $ = (id) => document.getElementById(id);
const panels = ["welcome", "idle", "model", "working", "paired", "guide"];
const show = (id) => panels.forEach((p) => $(p).classList.toggle("hidden", p !== id));
const setStatus = (cls, text) => {
  $("dot").className = "dot " + cls;
  $("statusText").textContent = text;
};

// True while the user is going through the first-run flow; lets us mark
// onboarding complete once the first pairing QR appears.
let onboarding = false;

// ---- First run: check the local model, then hand off to the share flow ----

async function checkModelServer() {
  show("working");
  setStatus("amber", "Checking your model…");
  let st;
  try {
    st = await invoke("model_server_status");
  } catch (e) {
    return modelPanel("Couldn’t check your model: " + e, null, null);
  }
  switch (st.state) {
    case "ready":
      return share(); // model present — continue to the secure link + QR
    case "not_running":
      return modelPanel(
        "Start Ollama, or start llama.cpp with its OpenAI-compatible server. llama.cpp ports <code>8080</code> and <code>8081</code> are detected automatically.",
        "llama.cpp setup", "https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md");
    case "no_model":
      return modelPanel(
        (st.backend === "llama_cpp"
          ? "llama.cpp is running, but it did not report a loaded model. Start <code>llama-server</code> with a GGUF model, then check again."
          : "Ollama is running, but there’s no model yet. Pull a model, then check again."),
        null, null);
    default:
      return modelPanel("Couldn’t check your model.", null, null);
  }
}

function modelPanel(html, btnLabel, btnUrl) {
  setStatus("red", "Action needed");
  $("modelText").innerHTML = html;
  const b = $("modelBtn");
  if (btnLabel) {
    b.textContent = btnLabel;
    b.classList.remove("hidden");
    b.onclick = () => btnUrl && invoke("open_url", { url: btnUrl });
  } else {
    b.classList.add("hidden");
  }
  show("model");
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
  if (!res.model_server_up || !res.model) return checkModelServer();

  const ts = res.tailscale;
  if (ts.state === "healthy" && res.qr_svg) {
    $("qr").innerHTML = res.qr_svg;
    const backend = res.backend === "llama_cpp" ? "llama.cpp" : "Ollama";
    $("modelLine").textContent = res.model ? "Sharing via " + backend + ": " + res.model : "";
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

$("startBtn").onclick = checkModelServer; // begin first-run setup at the model step
$("skipBtn").onclick = async () => {
  await invoke("set_onboarded");
  onboarding = false;
  setStatus("grey", "Not sharing yet");
  show("idle");
};
$("modelRecheck").onclick = checkModelServer;
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
