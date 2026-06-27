const DEFAULT_INSTANCE_ID = "saykocraft-earth";
const MAX_CONSOLE_LINES = 3000;
const AUTO_SCROLL_THRESHOLD_PX = 24;
const STORAGE_KEYS = Object.freeze({
    wrapLines: "saykocraft.devConsole.wrapLines",
    hideLauncherLogs: "saykocraft.devConsole.hideLauncherLogs",
});
const CONSOLE_CLASSES = Object.freeze({
    wrapLines: "console-wrap-lines",
    hideLauncherLogs: "console-hide-launcher-logs",
});
const ANSI_STRIP_PATTERN = /\x1b(?:\[[0-?]*[ -/]*[@-~]|\][\s\S]*?(?:\x07|\x1b\\)|[()*+\-./].)/g;
const ANSI_SGR_PATTERN = /\x1b\[([0-9;]*)m/g;
const LOG_LEVEL_PATTERN = /(?:^|[\s\[/])(?<level>trace|debug|info|warn|warning|error|fatal)(?=$|[\s\]/:])/i;
const LAUNCHER_LOG_PREFIX = "[SayKOCraft Launcher]";
const INHERITED_LOG_LEVELS = new Set(["warn", "error", "fatal"]);
const ANSI_COLORS = [
    "#000000",
    "#aa0000",
    "#ededed",
    "#eedc55",
    "#0000aa",
    "#aa00aa",
    "#00aaaa",
    "#aaaaaa",
];
const ANSI_BRIGHT_COLORS = [
    "#555555",
    "#ff5555",
    "#55ff55",
    "#ffff55",
    "#5555ff",
    "#ff55ff",
    "#55ffff",
    "#ffffff",
];

const consoleOutput = document.getElementById("console-output");
const clearConsoleButton = document.getElementById("console-clear");
const openLogFolderButton = document.getElementById("console-open-log-folder");
const wrapLinesInput = document.getElementById("console-wraplines");
const hideLauncherLogsInput = document.getElementById("console-hidelauncherlogs");
const query = new URLSearchParams(window.location.search);
let activeInstanceId = query.get("id") || DEFAULT_INSTANCE_ID;
let pendingLines = [];
let flushScheduled = false;
let inheritedConsoleLevel = "";

function queueConsoleLine(line) {
    if (!line || line.instanceId !== activeInstanceId) {
        return;
    }

    pendingLines.push(line);
    scheduleConsoleFlush();
}

function scheduleConsoleFlush() {
    if (flushScheduled) {
        return;
    }

    flushScheduled = true;
    requestAnimationFrame(flushConsoleLines);
}

function flushConsoleLines() {
    flushScheduled = false;

    if (!consoleOutput || pendingLines.length === 0) {
        pendingLines = [];
        return;
    }

    const scrollTarget = getConsoleScrollTarget();
    const shouldAutoScroll = isScrollTargetAtBottom(scrollTarget);
    const fragment = document.createDocumentFragment();

    for (const line of pendingLines) {
        const element = document.createElement("div");
        const text = String(line.line ?? "");
        const detectedLevel = detectConsoleLevel(text);
        const level = detectedLevel || inheritedConsoleLevel;
        const hasAnsiStyle = hasAnsiVisualStyle(text);
        const isLauncherLog = isSaykocraftLauncherLog(text);

        element.className = `console-line console-line-${line.stream}`;
        element.dataset.stream = line.stream;

        if (isLauncherLog) {
            element.classList.add("console-line-launcher");
            element.dataset.source = "launcher";
        }

        if (level) {
            element.dataset.level = level;
        }

        if (level && !hasAnsiStyle) {
            element.classList.add(`console-level-${level}`);
        }

        renderAnsiText(element, text);
        fragment.appendChild(element);
        updateInheritedConsoleLevel(text, detectedLevel, isLauncherLog);
    }

    pendingLines = [];
    consoleOutput.appendChild(fragment);
    trimConsoleLines();

    if (shouldAutoScroll) {
        scrollTargetToBottom(scrollTarget);
    }
}

function trimConsoleLines() {
    while (consoleOutput.children.length > MAX_CONSOLE_LINES) {
        consoleOutput.removeChild(consoleOutput.firstChild);
    }
}

function getConsoleScrollTarget() {
    return consoleOutput || document.scrollingElement || document.documentElement;
}

function isScrollTargetAtBottom(target) {
    if (!target) {
        return false;
    }

    const distanceFromBottom = Math.max(0, target.scrollHeight - target.scrollTop - target.clientHeight);
    return distanceFromBottom <= AUTO_SCROLL_THRESHOLD_PX;
}

function scrollTargetToBottom(target) {
    if (!target) {
        return;
    }

    target.scrollTop = target.scrollHeight;
}

function initializeLineWrapToggle() {
    initializePersistedCheckbox(wrapLinesInput, STORAGE_KEYS.wrapLines, setLineWrap);
}

function setLineWrap(wrapLines) {
    consoleOutput?.classList.toggle(CONSOLE_CLASSES.wrapLines, wrapLines);
}

function initializeLauncherLogToggle() {
    initializePersistedCheckbox(
        hideLauncherLogsInput,
        STORAGE_KEYS.hideLauncherLogs,
        setLauncherLogVisibility,
        { preserveBottomScroll: true },
    );
}

function setLauncherLogVisibility(hideLauncherLogs) {
    consoleOutput?.classList.toggle(CONSOLE_CLASSES.hideLauncherLogs, hideLauncherLogs);
}

function initializePersistedCheckbox(input, storageKey, applyValue, options = {}) {
    if (!input) {
        return;
    }

    const checked = localStorage.getItem(storageKey) === "true";
    input.checked = checked;
    applyValue(checked);

    input.addEventListener("change", () => {
        const scrollTarget = options.preserveBottomScroll ? getConsoleScrollTarget() : null;
        const shouldAutoScroll = scrollTarget ? isScrollTargetAtBottom(scrollTarget) : false;
        const nextValue = input.checked;

        applyValue(nextValue);
        localStorage.setItem(storageKey, String(nextValue));

        if (shouldAutoScroll) {
            scrollTargetToBottom(scrollTarget);
        }
    });
}

function initializeConsoleButtons() {
    clearConsoleButton?.addEventListener("click", () => {
        clearConsole().catch(console.error);
    });

    openLogFolderButton?.addEventListener("click", () => {
        openLogFolder().catch(console.error);
    });
}

async function clearConsole() {
    pendingLines = [];
    inheritedConsoleLevel = "";

    if (consoleOutput) {
        consoleOutput.textContent = "";
    }

    await invoke("clear_game_console_history", { id: activeInstanceId });
}

async function openLogFolder() {
    await invoke("open_game_log_folder", { id: activeInstanceId });
}

function isSaykocraftLauncherLog(text) {
    return text.replace(ANSI_STRIP_PATTERN, "").startsWith(LAUNCHER_LOG_PREFIX);
}

function updateInheritedConsoleLevel(text, detectedLevel, isLauncherLog) {
    if (isLauncherLog) {
        inheritedConsoleLevel = "";
        return;
    }

    if (detectedLevel) {
        inheritedConsoleLevel = INHERITED_LOG_LEVELS.has(detectedLevel) ? detectedLevel : "";
        return;
    }

    if (inheritedConsoleLevel && isStackTraceContinuation(text)) {
        return;
    }

    inheritedConsoleLevel = "";
}

function isStackTraceContinuation(text) {
    const plainText = text.replace(ANSI_STRIP_PATTERN, "");

    return (
        plainText.trim() === ""
        || /^\s/.test(plainText)
        || /^(?:at |Caused by:|Suppressed:|\.\.\. \d+ more\b)/.test(plainText)
        || /^[\w.$/]+(?:Exception|Error)(?::|\b)/.test(plainText)
        || /^[\w.-]+\.so:/.test(plainText)
        || /^Native library \(/.test(plainText)
    );
}

function hasAnsiVisualStyle(text) {
    for (const match of text.matchAll(ANSI_SGR_PATTERN)) {
        const codes = match[1].length === 0
            ? [0]
            : match[1]
                .split(";")
                .map((value) => value === "" ? 0 : Number(value))
                .filter(Number.isFinite);

        if (codes.some(isAnsiVisualStyleCode)) {
            return true;
        }
    }

    return false;
}

function isAnsiVisualStyleCode(code) {
    return (
        code === 1
        || code === 2
        || code === 3
        || code === 4
        || code === 38
        || code === 48
        || (code >= 30 && code <= 37)
        || (code >= 40 && code <= 47)
        || (code >= 90 && code <= 97)
        || (code >= 100 && code <= 107)
    );
}

function detectConsoleLevel(text) {
    const plainText = text.replace(ANSI_STRIP_PATTERN, "");
    const match = plainText.match(LOG_LEVEL_PATTERN);

    if (!match?.groups?.level) {
        return "";
    }

    const level = match.groups.level.toLowerCase();
    return level === "warning" ? "warn" : level;
}

function renderAnsiText(parent, text) {
    const state = createAnsiState();
    let chunk = "";
    let index = 0;

    while (index < text.length) {
        const character = text[index];

        if (character !== "\x1b") {
            chunk += character;
            index += 1;
            continue;
        }

        appendAnsiChunk(parent, chunk, state);
        chunk = "";

        const next = text[index + 1];
        if (next === "[") {
            index = consumeAnsiCsi(text, index + 2, state);
            continue;
        }

        if (next === "]") {
            index = consumeAnsiOsc(text, index + 2);
            continue;
        }

        if ("()*+-./".includes(next)) {
            index += 3;
            continue;
        }

        index += next ? 2 : 1;
    }

    appendAnsiChunk(parent, chunk, state);
}

function createAnsiState() {
    return {
        color: "",
        backgroundColor: "",
        fontWeight: "",
        fontStyle: "",
        textDecoration: "",
        opacity: "",
    };
}

function consumeAnsiCsi(text, index, state) {
    let sequence = "";

    while (index < text.length) {
        const character = text[index];
        index += 1;

        if (character >= "@" && character <= "~") {
            if (character === "m") {
                applyAnsiSgr(sequence, state);
            }
            return index;
        }

        sequence += character;
    }

    return index;
}

function consumeAnsiOsc(text, index) {
    while (index < text.length) {
        const character = text[index];
        index += 1;

        if (character === "\x07") {
            return index;
        }

        if (character === "\x1b" && text[index] === "\\") {
            return index + 1;
        }
    }

    return index;
}

function applyAnsiSgr(sequence, state) {
    const codes = sequence.length === 0
        ? [0]
        : sequence
            .split(";")
            .map((value) => value === "" ? 0 : Number(value))
            .filter(Number.isFinite);

    for (let index = 0; index < codes.length; index += 1) {
        const code = codes[index];

        if (code === 0) {
            resetAnsiState(state);
        } else if (code === 1) {
            state.fontWeight = "700";
        } else if (code === 2) {
            state.opacity = "0.75";
        } else if (code === 3) {
            state.fontStyle = "italic";
        } else if (code === 4) {
            state.textDecoration = "underline";
        } else if (code === 22) {
            state.fontWeight = "";
            state.opacity = "";
        } else if (code === 23) {
            state.fontStyle = "";
        } else if (code === 24) {
            state.textDecoration = "";
        } else if (code === 39) {
            state.color = "";
        } else if (code === 49) {
            state.backgroundColor = "";
        } else if (code >= 30 && code <= 37) {
            state.color = ANSI_COLORS[code - 30];
        } else if (code >= 40 && code <= 47) {
            state.backgroundColor = ANSI_COLORS[code - 40];
        } else if (code >= 90 && code <= 97) {
            state.color = ANSI_BRIGHT_COLORS[code - 90];
        } else if (code >= 100 && code <= 107) {
            state.backgroundColor = ANSI_BRIGHT_COLORS[code - 100];
        } else if (code === 38 || code === 48) {
            const color = parseAnsiExtendedColor(codes, index + 1);
            if (color) {
                if (code === 38) {
                    state.color = color.value;
                } else {
                    state.backgroundColor = color.value;
                }
                index = color.nextIndex - 1;
            }
        }
    }
}

function parseAnsiExtendedColor(codes, index) {
    const mode = codes[index];

    if (mode === 5 && Number.isInteger(codes[index + 1])) {
        return {
            value: ansi256Color(codes[index + 1]),
            nextIndex: index + 2,
        };
    }

    if (
        mode === 2
        && Number.isInteger(codes[index + 1])
        && Number.isInteger(codes[index + 2])
        && Number.isInteger(codes[index + 3])
    ) {
        return {
            value: `rgb(${clampColor(codes[index + 1])}, ${clampColor(codes[index + 2])}, ${clampColor(codes[index + 3])})`,
            nextIndex: index + 4,
        };
    }

    return null;
}

function ansi256Color(code) {
    code = Math.max(0, Math.min(255, code));

    if (code < 8) {
        return ANSI_COLORS[code];
    }

    if (code < 16) {
        return ANSI_BRIGHT_COLORS[code - 8];
    }

    if (code >= 232) {
        const value = 8 + (code - 232) * 10;
        return `rgb(${value}, ${value}, ${value})`;
    }

    const cube = code - 16;
    const red = Math.floor(cube / 36);
    const green = Math.floor((cube % 36) / 6);
    const blue = cube % 6;
    const component = (value) => value === 0 ? 0 : 55 + value * 40;

    return `rgb(${component(red)}, ${component(green)}, ${component(blue)})`;
}

function appendAnsiChunk(parent, text, state) {
    if (!text) {
        return;
    }

    if (!hasAnsiStyle(state)) {
        parent.appendChild(document.createTextNode(text));
        return;
    }

    const span = document.createElement("span");
    span.textContent = text;
    span.style.color = state.color;
    span.style.backgroundColor = state.backgroundColor;
    span.style.fontWeight = state.fontWeight;
    span.style.fontStyle = state.fontStyle;
    span.style.textDecoration = state.textDecoration;
    span.style.opacity = state.opacity;
    parent.appendChild(span);
}

function hasAnsiStyle(state) {
    return Boolean(
        state.color
        || state.backgroundColor
        || state.fontWeight
        || state.fontStyle
        || state.textDecoration
        || state.opacity
    );
}

function resetAnsiState(state) {
    state.color = "";
    state.backgroundColor = "";
    state.fontWeight = "";
    state.fontStyle = "";
    state.textDecoration = "";
    state.opacity = "";
}

function clampColor(value) {
    return Math.max(0, Math.min(255, value));
}

function queueConsoleStatus(status) {
    if (!status || status.instanceId !== activeInstanceId) {
        return;
    }

    let line = `[SayKOCraft Launcher] Minecraft ${status.status}`;

    if (status.status === "started" && status.pid) {
        line = `[SayKOCraft Launcher] Minecraft started with pid ${status.pid}`;
    }

    if (status.status === "exited") {
        line = `[SayKOCraft Launcher] Minecraft exited with code ${status.exitCode ?? "unknown"}`;
    }

    queueConsoleLine({
        instanceId: status.instanceId,
        stream: "system",
        line,
    });
}

async function loadConsoleHistory() {
    const history = await invoke("get_game_console_history", { id: activeInstanceId });
    for (const line of history) {
        queueConsoleLine(line);
    }
}

async function setConsoleInstance(id) {
    activeInstanceId = id || DEFAULT_INSTANCE_ID;
    pendingLines = [];
    inheritedConsoleLevel = "";

    if (consoleOutput) {
        consoleOutput.textContent = "";
    }

    await loadConsoleHistory();
}

async function initializeDevConsole() {
    initializeLineWrapToggle();
    initializeLauncherLogToggle();
    initializeConsoleButtons();
    await loadConsoleHistory();

    if (!tauriEvent?.listen) {
        return;
    }

    await tauriEvent.listen("minecraft-console-line", (event) => {
        queueConsoleLine(event.payload);
    });

    await tauriEvent.listen("minecraft-console-status", (event) => {
        queueConsoleStatus(event.payload);
    });

    await tauriEvent.listen("minecraft-console-instance-selected", (event) => {
        setConsoleInstance(event.payload).catch(console.error);
    });
}

initializeDevConsole().catch(console.error);
