const usernameTooltip = document.getElementById("sayko-username");
const profileIcon = document.getElementById("sayko-profileicon");
const centerPages = document.querySelectorAll(".centerpage");
const launcherSettingsButton = document.querySelector(".sideBar .icon-settings");
const serverButtons = document.querySelectorAll(".serverlist .server");
let lastSelectedServer = document.querySelector(".serverlist .server.selected") || null;

const modpackActionButton = document.getElementById("modpack-actionbutton");
const modpackActionButtonLabel = document.getElementById("modpack-actionbutton-label");

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

    modpackActionButton.classList.remove("disabled");
    modpackActionButton.classList.remove("start");
    modpackActionButton.classList.remove("stop");
    modpackActionButton.classList.remove("update");
    modpackActionButton.classList.remove("download");

    switch(Object.values(InstanceState)[buttonState]) {
        case InstanceState.Unknown:
        case InstanceState.Broken: {
            modpackActionButton.classList.add("disabled");
            modpackActionButtonLabel.textContent = t("action.broken");
            break;
        }
        case InstanceState.RequiresUpdate: {
            modpackActionButton.classList.add("update");
            modpackActionButtonLabel.textContent = t("action.update");
            console.log(t("action.update"))
            break;
        }
        case InstanceState.Ready: {
            modpackActionButton.classList.add("start");
            modpackActionButtonLabel.textContent = t("action.start");
            break;
        }
        case InstanceState.Launched: {
            modpackActionButton.classList.add("stop");
            modpackActionButtonLabel.textContent = t("action.stop");
            break;
        }
        case InstanceState.NotInstalled: {
            modpackActionButton.classList.add("download");
            modpackActionButtonLabel.textContent = t("action.download");
            break;
        }
    }
}

async function initializeLauncher() {
    await initializeLocalization();

    setUsername();
    setProfileIcon();
    setLauncherVersion();
    setLauncherView(LauncherView.MAIN);
    setModpackButton();
}

initializeLauncher().catch(console.error);
