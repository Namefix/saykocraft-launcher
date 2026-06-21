const { invoke } = window.__TAURI__.core;
const { openUrl } = window.__TAURI__.opener;
const { listen } = window.__TAURI__.event;

const InstanceState = Object.freeze({
    Unknown: 0,
    NotInstalled: 1,
    RequiresUpdate: 2,
    Ready: 3,
    Launched: 4,
    Broken: 5
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