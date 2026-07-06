const { invoke } = window.__TAURI__.core;
const { openUrl } = window.__TAURI__.opener;
const { listen } = window.__TAURI__.event;
const tauriEvent = window.__TAURI__?.event;

const pressedKeys = new Set();

function getUserTimeZone() {
    try {
        return Intl.DateTimeFormat().resolvedOptions().timeZone || undefined;
    } catch {
        return undefined;
    }
}

function epochToDate(value) {
    const epoch = Number(value);
    if (!Number.isFinite(epoch) || epoch <= 0) {
        return null;
    }

    const timestamp = epoch > 9999999999 ? epoch : epoch * 1000;
    const date = new Date(timestamp);
    if (Number.isNaN(date.getTime())) {
        return null;
    }

    return date;
}

function formatEpochDateTime(value, options = {}) {
    const date = epochToDate(value);
    if (!date) {
        return null;
    }

    const locale = options.locale ?? document.documentElement.lang ?? undefined;
    const timeZone = options.timeZone ?? getUserTimeZone();

    return new Intl.DateTimeFormat(locale, {
        dateStyle: options.dateStyle ?? "medium",
        timeStyle: options.timeStyle ?? "short",
        ...(timeZone ? { timeZone } : {})
    }).format(date);
}

function isReloadShortcut(e) {
    const key = e.key?.toLowerCase();

    return e.code === "F5" ||
        key === "f5" ||
        ((e.ctrlKey || e.metaKey) && key === "r");
}

function preventReloadShortcut(e) {
    e.preventDefault();
    e.stopImmediatePropagation();
}

function isEditableContextTarget(target) {
    return Boolean(target?.closest?.(
        "input:not([disabled]):not([readonly]), textarea:not([disabled]):not([readonly]), [contenteditable='true']"
    ));
}

async function openWebviewDevtools() {
    try {
        await invoke("open_webview_devtools");
    } catch (err) {
        console.error("Failed to open webview devtools.", err);
    }
}

document.addEventListener("contextmenu", (e) => {
    if (isEditableContextTarget(e.target)) {
        return;
    }

    e.preventDefault();
}, { capture: true });

document.addEventListener("keydown", (e) => {
    if (isReloadShortcut(e)) {
        preventReloadShortcut(e);
        return;
    }

    if (e.code === "F12" || e.key === "F12") {
        e.preventDefault();
        openWebviewDevtools();
        return;
    }

    pressedKeys.add(e.code);
}, { capture: true });

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
    openUrl("https://nf.blacksmith-ent.com/auth/password")
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
