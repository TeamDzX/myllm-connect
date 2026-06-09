const invoke = window.__TAURI__.core.invoke;

const $ = (id) => document.getElementById(id);
const panels = ["idle", "working", "paired", "guide"];
const show = (id) => panels.forEach((p) => $(p).classList.toggle("hidden", p !== id));
const setStatus = (cls, text) => {
  $("dot").className = "dot " + cls;
  $("statusText").textContent = text;
};

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
  const ts = res.tailscale;
  if (ts.state === "healthy" && res.qr_svg) {
    $("qr").innerHTML = res.qr_svg;
    $("modelLine").textContent = res.model ? "Sharing: " + res.model : "";
    setStatus("green", "Ready to pair");
    show("paired");
    return;
  }
  switch (ts.state) {
    case "not_installed":
      return guide(
        "MyLLM Connect needs Tailscale to create a secure link to your phone. Install it once, sign in, then try again.",
        "Get Tailscale", "https://tailscale.com/download");
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
