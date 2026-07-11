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

initializeLocalization();

function showLoginError(message) {
    errorMessage.classList.remove("hidden");
    errorMessage.textContent = message;
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
        return {
            code: err.code ?? null,
            message: err.message ?? null,
            expiresAt: err.expires_at ?? err.expiresAt ?? null
        };
    }

    const raw = typeof err === "string" ? err : err?.message;
    if (typeof raw === "string") {
        try {
            const parsed = JSON.parse(raw);
            return {
                code: parsed.code ?? null,
                message: parsed.message ?? raw,
                expiresAt: parsed.expires_at ?? parsed.expiresAt ?? null
            };
        } catch {
            return { code: null, message: raw, expiresAt: null };
        }
    }

    return { code: null, message: null, expiresAt: null };
}

function normalizeSessionStatus(payload) {
    if (typeof payload === "string") {
        return { status: payload, error: null };
    }

    if (payload && typeof payload === "object") {
        return {
            status: payload.status ?? null,
            error: payload.error ?? null
        };
    }

    return { status: null, error: null };
}

function getLoginErrorMessage(code, message, expiresAt) {
    switch (code) {
        case "INVALID_CREDENTIALS":
            return t("error.loginInvalidCredentials");
        case "ACCOUNT_NOT_APPROVED":
            return t("error.loginAccountNotApproved");
        case "BANNED": {
            const formattedExpiresAt = formatEpochDateTime(expiresAt);
            if (formattedExpiresAt) {
                return t("error.loginBannedUntil", { expiresAt: formattedExpiresAt });
            }

            return t("error.loginBanned");
        }
        case "RATE_LIMITED":
            return t("error.loginRateLimited");
        case "AUTH_FAILED":
            return t("error.loginAuthFailed");
        case "UPGRADE_REQUIRED":
            return t("error.loginUpgradeRequired");
        case "SERVICE_UNAVAILABLE":
            return t("error.loginMaintenance");
        case "NETWORK_ERROR":
            return t("error.loginNetworkError");
        case "SERVER_ERROR":
            return t("error.loginServerError");
        default:
            return message || t("error.loginUnknownError");
    }
}

function showAuthError(err) {
    const { code, message, expiresAt } = normalizeInvokeError(err);
    showLoginError(getLoginErrorMessage(code, message, expiresAt));
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
    const { status, error } = normalizeSessionStatus(event.payload);

    if (status === "valid") {
        openLauncher();
        return;
    }

    if (status === "network-error") {
        showSessionError(t("error.networkError"));
        return;
    }

    if (status === "error") {
        showLoginScreen();
        showAuthError(error);
        return;
    }

    if(status === "invalid" || status === "null") {
        showLoginScreen();
        if(status === "invalid") {
            showLoginError(t("error.sessionExpired"));
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
        showSessionError(t("error.networkError"));
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
        showAuthError(e);
    } finally {
        setLoginPending(false);
    }
}

addEventListener("keyup", (e) => {
    if(e.code == "Enter" && !loginButton.classList.contains("is-loading")) {
        login()
    }
})
