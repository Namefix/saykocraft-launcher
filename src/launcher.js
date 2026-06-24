const usernameTooltip = document.getElementById("sayko-username");
const profileIcon = document.getElementById("sayko-profileicon");
const centerPages = document.querySelectorAll(".centerpage");
const launcherSettingsButton = document.querySelector(".sideBar .icon-settings");
const serverButtons = document.querySelectorAll(".serverlist .server");
let lastSelectedServer = document.querySelector(".serverlist .server.selected") || null;

const modpackActionButton = document.getElementById("modpack-actionbutton");
const modpackActionButtonLabel = document.getElementById("modpack-actionbutton-label");

const launcherProgressBar = document.getElementById("launcher-progressbar");
const launcherLoadingBar = document.getElementById("launcher-loadingbar");

const launcherSettingsVersionText = document.getElementById("sayko-launcherversion");

const LauncherView = Object.freeze({
    MAIN: "MAIN",
    LAUNCHER_SETTINGS: "LAUNCHER_SETTINGS",
    MODPACK_SETTINGS: "MODPACK_SETTINGS"
});

launcherSettingsButton?.addEventListener("click", () => {
    setLauncherView(LauncherView.LAUNCHER_SETTINGS);
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
async function setModpackButton() {
    let buttonState = await invoke("get_instance_state", {id:"saykocraft-earth"});

    switch(Object.values(InstanceState)[buttonState]) {
        case InstanceState.Unknown:
        case InstanceState.Broken: {
            setActionButtonState(null, t("action.broken"), true);
            break;
        }
        case InstanceState.RequiresUpdate: {
            setActionButtonState("update", t("action.update"));
            break;
        }
        case InstanceState.Updating: {
            setActionButtonState("update", t("action.updating"), true);
            break;
        }
        case InstanceState.Ready: {
            setActionButtonState("start", t("action.start"));
            break;
        }
        case InstanceState.Launched: {
            setActionButtonState("stop", t("action.stop"));
            break;
        }
        case InstanceState.NotDownloaded: {
            setActionButtonState("download", t("action.download"));
            break;
        }
        case InstanceState.Downloading: {
            setActionButtonState("download", t("action.downloading"), true);
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
}

async function actionButtonHandler() {
    let buttonState = await invoke("get_instance_state", {id:"saykocraft-earth"});

    switch(Object.values(InstanceState)[buttonState]) {
        case InstanceState.RequiresUpdate:
        case InstanceState.NotDownloaded: {
            ensureInstance("saykocraft-earth");
            break;
        }
        case InstanceState.RequiresUpdate: {
            break;
        }
        case InstanceState.Ready: {
            startInstance("saykocraft-earth");
            break;
        }
        case InstanceState.Launched: {
            stopInstance("saykocraft-earth");
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

async function ensureInstance(id) {
    console.log("Downloading instance", id);

    let result = await invoke("ensure_instance", {id});
    console.log("Instance downloaded result", result);
    setProgressBarPercentage(0);
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

async function setEventListeners() {
    if(tauriEvent?.listen) {
        tauriEvent.listen("instance-install-progress", (event) => {
            const progress = event.payload;

            setProgressBarPercentage(progress.overall_percentage ?? 0);
        });

        tauriEvent.listen("instance-state-changed", (event) => {
            const { id, state, stateCode } = event.payload;
            setModpackButton();
        });
    }
}

async function initializeLauncher() {
    await initializeLocalization();

    setUsername();
    setProfileIcon();
    setLauncherVersion();
    setLauncherView(LauncherView.MAIN);
    setModpackButton();
    setEventListeners();
}

initializeLauncher().catch(console.error);
