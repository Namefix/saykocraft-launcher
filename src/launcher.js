const ACTIVE_INSTANCE_ID = "saykocraft-earth";
const usernameTooltip = document.getElementById("sayko-username");
const profileIcon = document.getElementById("sayko-profileicon");
const centerPages = document.querySelectorAll(".centerpage");
const launcherSettingsButton = document.querySelector(".sideBar .icon-settings");
const serverButtons = document.querySelectorAll(".serverlist .server");
let lastSelectedServer = document.querySelector(".serverlist .server.selected") || null;

const modpackActionButton = document.getElementById("modpack-actionbutton");
const modpackActionButtonLabel = document.getElementById("modpack-actionbutton-label");
const modpackActionTooltip = document.getElementById("modpack-action-tooltip");
const modpackActionTooltipTitle = document.getElementById("modpack-action-tooltip-title");
const modpackActionTooltipDescription = document.getElementById("modpack-action-tooltip-description");
const modpackSettingsButton = document.getElementById("modpack-settingsbutton");
const modpackSettingsBackButton = document.getElementById("modpacksettings-back");

const launcherProgressBar = document.getElementById("launcher-progressbar");
const launcherLoadingBar = document.getElementById("launcher-loadingbar");

const launcherSettingsVersionText = document.getElementById("sayko-launcherversion");
const modpackVersionText = document.getElementById("modpack-version");
const modpackFileSizeText = document.getElementById("modpack-filesize");
let actionButtonTooltipEnabled = false;
let actionButtonTooltipHovered = false;

const LauncherView = Object.freeze({
    MAIN: "MAIN",
    LAUNCHER_SETTINGS: "LAUNCHER_SETTINGS",
    MODPACK_SETTINGS: "MODPACK_SETTINGS"
});

launcherSettingsButton?.addEventListener("click", () => {
    setLauncherView(LauncherView.LAUNCHER_SETTINGS);
});

modpackSettingsButton?.addEventListener("click", async () => {
    let instanceState = await invoke("get_instance_state", {id: ACTIVE_INSTANCE_ID});
    let state = Object.values(InstanceState)[instanceState];

    if (state === InstanceState.Ready || state === InstanceState.RequiresUpdate) {
        await setModpackSettings();
        setLauncherView(LauncherView.MODPACK_SETTINGS);
    }
});

modpackSettingsBackButton?.addEventListener("click", () => {
    setLauncherView(LauncherView.MAIN);
});

modpackActionButton?.addEventListener("mouseenter", (event) => {
    actionButtonTooltipHovered = true;
    updateActionButtonTooltipPosition(event);
    syncActionButtonTooltipVisibility();
});

modpackActionButton?.addEventListener("mousemove", (event) => {
    updateActionButtonTooltipPosition(event);
});

modpackActionButton?.addEventListener("mouseleave", () => {
    actionButtonTooltipHovered = false;
    syncActionButtonTooltipVisibility();
});

serverButtons.forEach((server) => {
    server.addEventListener("click", () => {
        if (server.classList.contains("unselectable")) {
            return;
        }

        setSelectedServer(server);
        setLauncherView(LauncherView.MAIN);
    });
});

function setLauncherView(view) {
    centerPages.forEach((page) => {
        const isActive = page.dataset.page === view;
        page.classList.toggle("is-active", isActive);
        page.setAttribute("aria-hidden", (!isActive).toString());
    });

    applySidebarSelection(view);
}

function setSelectedServer(server) {
    if (!server || server.classList.contains("unselectable")) {
        return;
    }

    lastSelectedServer = server;
    serverButtons.forEach((button) => {
        button.classList.toggle("selected", button === server);
    });
}

function applySidebarSelection(view) {
    if (view === LauncherView.LAUNCHER_SETTINGS) {
        launcherSettingsButton?.classList.add("selected");
        serverButtons.forEach((button) => button.classList.remove("selected"));
        return;
    }

    launcherSettingsButton?.classList.remove("selected");
    if (!lastSelectedServer) {
        lastSelectedServer = Array.from(serverButtons).find(
            (button) => !button.classList.contains("unselectable")
        ) || null;
    }

    if (lastSelectedServer) {
        setSelectedServer(lastSelectedServer);
    }
}

async function setUsername() {
    let username = await invoke("get_username");
    usernameTooltip.textContent = username;
}
async function setProfileIcon() {
    let icon;
    try {
        icon = await invoke("get_profile_icon");
        profileIcon.style = `background-image: url("${icon}")`
    } catch (err) {
        console.error("Failed to fetch username.", err);
    }
}
async function setLauncherVersion() {
    let version = await invoke("get_launcher_version");
    launcherSettingsVersionText.textContent = `saykocraft-launcher v${version}`
}
async function setModpackVersion() {
    try {
        let version = await invoke("get_instance_version", {id: ACTIVE_INSTANCE_ID});
        modpackVersionText.textContent = `v${version}`;
    } catch (err) {
        console.error("Failed to fetch modpack version.", err);
        modpackVersionText.textContent = "";
    }
}
async function setModpackFileSize() {
    if (!modpackFileSizeText) {
        return;
    }

    modpackFileSizeText.textContent = t("filesize.calculating");

    try {
        let bytes = await invoke("get_instance_folder_size", {id: ACTIVE_INSTANCE_ID});
        modpackFileSizeText.textContent = formatFileSize(bytes);
    } catch (err) {
        console.error("Failed to calculate modpack file size.", err);
        modpackFileSizeText.textContent = t("filesize.unavailable");
    }
}
async function setModpackButtons() {
    let buttonState = await invoke("get_instance_state", {id: ACTIVE_INSTANCE_ID});

    switch(Object.values(InstanceState)[buttonState]) {
        case InstanceState.Unknown:
        case InstanceState.Broken: {
            setActionButtonState(null, t("action.broken"), true);
            setModpackSettingsButtonState(false);
            break;
        }
        case InstanceState.RequiresUpdate: {
            setActionButtonState("update", t("action.update"));
            setModpackSettingsButtonState(false);
            break;
        }
        case InstanceState.Updating: {
            setActionButtonState("update", t("action.updating"), true);
            setActionButtonTooltip(
                t("actionTooltip.updatingTitle"),
                t("actionTooltip.updatingDescription")
            );
            setActionButtonTooltipEnabled(true);
            setModpackSettingsButtonState(true);
            break;
        }
        case InstanceState.Ready: {
            setActionButtonState("start", t("action.start"));
            setModpackSettingsButtonState(false);
            break;
        }
        case InstanceState.Launched: {
            setActionButtonState("stop", t("action.stop"));
            setModpackSettingsButtonState(true);
            break;
        }
        case InstanceState.NotDownloaded: {
            setActionButtonState("download", t("action.download"));
            setModpackSettingsButtonState(true);
            break;
        }
        case InstanceState.Downloading: {
            setActionButtonState("download", t("action.downloading"), true);
            setActionButtonTooltip(
                t("actionTooltip.downloadingTitle"),
                t("actionTooltip.downloadingDescription")
            );
            setActionButtonTooltipEnabled(true);
            setModpackSettingsButtonState(true);
            break;
        }
    }
}

function setActionButtonState(classname, labeltext, disabled=false) {
    modpackActionButton.classList.remove("disabled");
    modpackActionButton.classList.remove("start");
    modpackActionButton.classList.remove("stop");
    modpackActionButton.classList.remove("update");
    modpackActionButton.classList.remove("download");
    
    if(classname) modpackActionButton.classList.add(classname);
    if(disabled) modpackActionButton.classList.add("disabled");
    modpackActionButtonLabel.textContent = labeltext;
    setActionButtonTooltipEnabled(false);
}

function setActionButtonTooltip(title, description) {
    if (modpackActionTooltipTitle) {
        modpackActionTooltipTitle.textContent = title ?? "";
    }

    if (modpackActionTooltipDescription) {
        modpackActionTooltipDescription.textContent = description ?? "";
    }
}

function setActionButtonTooltipEnabled(enabled) {
    actionButtonTooltipEnabled = Boolean(enabled);
    syncActionButtonTooltipVisibility();
}

function syncActionButtonTooltipVisibility() {
    if (!modpackActionTooltip) {
        return;
    }

    const hasContent = Boolean(
        modpackActionTooltipTitle?.textContent ||
        modpackActionTooltipDescription?.textContent
    );
    const visible = actionButtonTooltipEnabled && actionButtonTooltipHovered && hasContent;

    modpackActionTooltip.classList.toggle("visible", visible);
    modpackActionTooltip.setAttribute("aria-hidden", (!visible).toString());
}

function updateActionButtonTooltipPosition(event) {
    if (!modpackActionTooltip || !event) {
        return;
    }

    const offset = 18;
    const tooltipWidth = modpackActionTooltip.offsetWidth || 240;
    const tooltipHeight = modpackActionTooltip.offsetHeight || 64;
    const maxX = window.innerWidth - tooltipWidth - 8;
    const maxY = window.innerHeight - tooltipHeight - 8;
    const x = Math.max(8, Math.min(event.clientX + offset, maxX));
    const y = Math.max(8, Math.min(event.clientY + offset, maxY));

    modpackActionTooltip.style.setProperty("--tooltip-x", `${x}px`);
    modpackActionTooltip.style.setProperty("--tooltip-y", `${y}px`);
}

function setModpackSettingsButtonState(disabled, warning=false) {
    modpackSettingsButton.classList.remove("disabled");
    modpackSettingsButton.classList.remove("warning");

    if(disabled) modpackSettingsButton.classList.add("disabled");
    if(warning) modpackSettingsButton.classList.add("warning");
}

function formatFileSize(bytes) {
    const units = ["B", "KB", "MB", "GB", "TB"];
    let size = Number(bytes);
    let unitIndex = 0;

    while(size >= 1024 && unitIndex < units.length - 1) {
        size /= 1024;
        unitIndex++;
    }

    let maximumFractionDigits = unitIndex === 0 ? 0 : 1;
    return `${size.toLocaleString(undefined, { maximumFractionDigits })} ${units[unitIndex]}`;
}

async function actionButtonHandler() {
    let buttonState = await invoke("get_instance_state", {id: ACTIVE_INSTANCE_ID});

    // Forced launcher start, not advised unless absolutely necessary.
    if(pressedKeys.has("KeyN") && pressedKeys.has("KeyF") && pressedKeys.has("KeyG")) {
        buttonState = InstanceState.Ready;
    }

    switch(Object.values(InstanceState)[buttonState]) {
        case InstanceState.RequiresUpdate:
        case InstanceState.NotDownloaded: {
            ensureInstance(ACTIVE_INSTANCE_ID);
            break;
        }
        case InstanceState.Ready: {
            startInstance(ACTIVE_INSTANCE_ID);
            break;
        }
        case InstanceState.Launched: {
            stopInstance(ACTIVE_INSTANCE_ID);
            break;
        }
    }
}

function setProgressBarPercentage(value) {
    if(value > 0) {
        launcherLoadingBar.classList.add("active");
    } else {
        launcherLoadingBar.classList.remove("active");
    }

    launcherProgressBar.style = `width: ${value}%`;
}

async function ensureInstance(id="saykocraft-earth") {
    console.log("Downloading instance", id);

    let result = await invoke("ensure_instance", {id});
    console.log("Instance downloaded result", result);
    setProgressBarPercentage(0);
    setModpackVersion();
    setModpackFileSize();
    setModpackSettings();
}

async function startInstance(id) {
    console.log("Starting instance", id);

    let result = await invoke("launch_instance", {id});
    console.log("Instance exited", result);
}

async function stopInstance(id) {
    console.log("Stopping instance", id);

    await invoke("stop_instance", {id});
}

async function browseInstance(id) {
    console.log("Browsing instance", id);

    try {
        await invoke("browse_instance", {id});
    } catch (err) {
        console.error("Failed to browse instance.", err);
    }
}

async function removeInstance(id="saykocraft-earth") {
    console.log("Removing instance", id);

    let result = await invoke("remove_instance", {id});
    setLauncherView(LauncherView.MAIN);
    console.log("Instance removed", result);
}

async function openDevConsole(id=ACTIVE_INSTANCE_ID) {
    try {
        await invoke("open_dev_console", {id});
    } catch (err) {
        console.error("Failed to open developer console.", err);
    }
}

async function setEventListeners() {
    if(tauriEvent?.listen) {
        tauriEvent.listen("instance-install-progress", (event) => {
            const progress = event.payload;

            setActionButtonTooltip(t("installphase."+progress.phase), progress.current_label);

            setProgressBarPercentage(progress.overall_percentage ?? 0);
        });

        tauriEvent.listen("instance-state-changed", (event) => {
            const { id, state, stateCode } = event.payload;
            setModpackButtons();
            setModpackFileSize();
            if (id === ACTIVE_INSTANCE_ID && document.querySelector(".centerpage-modpacksettings.is-active")) {
                setModpackSettings();
            }
        });
    }
}

async function initializeLauncher() {
    await initializeLocalization();

    setUsername();
    setProfileIcon();
    setLauncherVersion();
    setModpackVersion();
    setModpackFileSize();
    setLauncherView(LauncherView.MAIN);
    setModpackButtons();
    setEventListeners();
}

initializeLauncher().catch(console.error);
