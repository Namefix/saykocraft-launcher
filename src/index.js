const tauriEvent = window.__TAURI__?.event;

const loadingDiv = document.getElementById("loadingDiv");
const centerDiv = document.getElementById("centerDiv");
const errorMessage = document.getElementById("errorMessage");
const sessionError = document.getElementById("sessionError");
const sessionErrorMessage = sessionError?.querySelector(".session-error-message");
const loginButton = document.getElementById("loginButton");
const loginButtonLabel = loginButton?.querySelector(".button-label");
const loginButtonSpinner = loginButton?.querySelector(".button-spinner");

const usernameInput = document.getElementById("sayko-username");
const passwordInput = document.getElementById("sayko-password");

function showLoginError(message) {
    errorMessage.classList.remove("hidden");
    errorMessage.innerHTML = message;
}

function clearSessionError() {
    sessionError?.classList.add("hidden");
    loadingDiv?.classList.remove("has-error");
}

function showSessionError(message) {
    loadingDiv.classList.remove("hidden");
    centerDiv.classList.add("hidden");

    if (sessionErrorMessage) {
        sessionErrorMessage.textContent = message;
    }

    sessionError?.classList.remove("hidden");
    loadingDiv?.classList.add("has-error");
}

function showSessionLoading() {
    clearSessionError();
    loadingDiv.classList.remove("hidden");
    centerDiv.classList.add("hidden");
}

function showLoginScreen() {
    clearSessionError();
    loadingDiv.classList.add("hidden");
    centerDiv.classList.remove("hidden");
}

function setLoginPending(isPending) {
    if (!loginButton) return;

    loginButton.disabled = isPending;
    loginButton.classList.toggle("is-loading", isPending);

    loginButtonLabel?.classList.toggle("hidden", isPending);
    loginButtonSpinner?.classList.toggle("hidden", !isPending);
}

function normalizeInvokeError(err) {
    if (err && typeof err === "object" && "code" in err) {
        return { code: err.code ?? null, message: err.message ?? null };
    }

    const raw = typeof err === "string" ? err : err?.message;
    if (typeof raw === "string") {
        try {
            const parsed = JSON.parse(raw);
            return { code: parsed.code ?? null, message: parsed.message ?? raw };
        } catch {
            return { code: null, message: raw };
        }
    }

    return { code: null, message: null };
}

async function openLauncher() {
    try {
        await invoke("set_launcher_window");
    } catch (e) {
        console.error("Failed to resize window", e);
    }

    window.location.replace("launcher.html");
}

if (tauriEvent?.listen && invoke) {
  tauriEvent.listen("session-status", (event) => {
    if (event.payload === "valid") {
        openLauncher();
        return;
    }

    if (event.payload === "network-error") {
        showSessionError("Network error. Please check your connection and try again.");
        return;
    }

    if(event.payload === "invalid" || event.payload === "null") {
        showLoginScreen();
        if(event.payload === "invalid") {
            showLoginError("Your session has expired.");
        } else {
            errorMessage.classList.add("hidden");
        }
    }
    }).then(() => {
        if (sessionStorage.getItem("skip-session-check") === "1") {
                sessionStorage.removeItem("skip-session-check");
                showLoginScreen();
                return;
        }

        return invoke("check_session");
    }).catch(console.error);
}

async function retrySession() {
    showSessionLoading();

    try {
        await invoke("check_session");
    } catch (e) {
        console.error("Failed to check session", e);
        showSessionError("Network error. Please check your connection and try again.");
    }
}

async function login() {
    let username = usernameInput.value;
    let password = passwordInput.value;

    if(username == null || username.trim().length === 0) return;
    if(password == null || password.trim().length === 0) return;

    errorMessage.classList.add("hidden");
    setLoginPending(true);

    try {
        await invoke("login", { username, password });
        await openLauncher();
    } catch (e) {
        const { code, message } = normalizeInvokeError(e);
        switch (code) {
            case "INVALID_CREDENTIALS":
                showLoginError("Username or password is incorrect!");
                break;
            case "RATE_LIMITED":
                showLoginError("Too many requests. Please wait and try again.");
                break;
            case "AUTH_FAILED":
                showLoginError("Login failed. Please try again.");
                break;
            case "UPGRADE_REQUIRED":
                showLoginError("Launcher is outdated. Please update and try again.");
                break;
            case "NETWORK_ERROR":
                showLoginError("Network error. Please check your connection.");
                break;
            case "SERVER_ERROR":
                showLoginError("Server error. Please try again later.");
                break;
            default:
                showLoginError(message || "Unknown error");
        }
    } finally {
        setLoginPending(false);
    }
}

addEventListener("keyup", (e) => {
    if(e.code == "Enter" && !loginButton.classList.contains("is-loading")) {
        login()
    }
})