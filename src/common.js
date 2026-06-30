const { invoke } = window.__TAURI__.core;
const { openUrl } = window.__TAURI__.opener;
const { listen } = window.__TAURI__.event;
const tauriEvent = window.__TAURI__?.event;

const pressedKeys = new Set();

document.addEventListener("keydown", (e) => {
    pressedKeys.add(e.code);
});

document.addEventListener("keyup", (e) => {
    pressedKeys.delete(e.code);
});

document.addEventListener("blur", () => {
    pressedKeys.clear();
});

const InstanceState = Object.freeze({
    Unknown: 0,
    NotDownloaded: 1,
    Downloading: 2,
    RequiresUpdate: 3,
    Updating: 4,
    Ready: 5,
    Launched: 6,
    Broken: 7
});

function onClose(e) {
    e?.stopPropagation();
    invoke("window_close");
}

function onMinimize(e) {
    e?.stopPropagation();
    invoke("window_minimize");
}

function openDiscord() {
    openUrl("https://discord.gg/8JFSGwR5Hv")
}

function openPasswordForget() {
    openUrl("https://google.com")
}

function reset_data() {
    invoke("reset_data");
}

async function get_config() {
    return await invoke("get_config");
}

async function logout() {
    sessionStorage.setItem("skip-session-check", "1");

    try {
        await invoke("stop_instance", {id:"saykocraft-earth"});
    } catch {}

    try {
        await invoke("reset_data");
    } catch (e) {
        console.error("Failed to reset data", e);
    }

    try {
        await invoke("set_login_window");
    } catch (e) {
        console.error("Failed to resize window", e);
    }

    window.location.replace("index.html");
}