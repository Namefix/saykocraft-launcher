const configInputs = document.querySelectorAll("select[data-config-key], input[data-config-key]");
const instanceConfigInputs = document.querySelectorAll("[data-instance-config-key]");
const modpackLocationInput = document.getElementById("modpack-location");
const CONFIG_PATH_VARIABLE = "$SAYKOCRAFT";
const ABSOLUTE_PATH_REGEX = /^(?:[A-Za-z]:[\\/]|[\\/])/u;
const INVALID_PATH_CHARACTER_REGEX = /[\0-\x1F<>:"|?*]/u;
const CONTROL_CHARACTER_REGEX = /[\0-\x1F\x7F]/u;

function isValidPathValue(value) {
    if (!value.trim() || value.includes("\0")) {
        return false;
    }

    if (value === CONFIG_PATH_VARIABLE) {
        return true;
    }

    if (value.startsWith(CONFIG_PATH_VARIABLE)) {
        let remainder = value.slice(CONFIG_PATH_VARIABLE.length);

        if (!remainder.startsWith("/") && !remainder.startsWith("\\")) {
            return false;
        }

        return remainder.split(/[\\/]/u).every(component => {
            return component === "" ||
                component === "." ||
                (component !== ".." && !INVALID_PATH_CHARACTER_REGEX.test(component));
        });
    }

    if (value.includes(CONFIG_PATH_VARIABLE)) {
        return false;
    }

    let valueWithoutWindowsDrive = value.replace(/^[A-Za-z]:/u, "");
    return ABSOLUTE_PATH_REGEX.test(value) && !INVALID_PATH_CHARACTER_REGEX.test(valueWithoutWindowsDrive);
}

function registerConfigListeners() {
    configInputs.forEach(config => {
        config.addEventListener("change", inputConfigImpl);
    });
}

async function setInitialConfigValues(configValues) {
    if (!configValues) {
        return;
    }

    configInputs.forEach(element => {
        let key = getConfigKey(element);

        if (!Object.prototype.hasOwnProperty.call(configValues, key)) {
            return;
        }

        setElementValue(element, configValues[key]);
        setLastSuccessfulValue(element, configValues[key]);
    });
}

function getElementInputType(e) {
    if (e.localName === "input") {
        return e.getAttribute("type");
    }
    return e.localName;
}

function getConfigKey(element) {
    return element.dataset.configKey;
}

function getElementValue(element) {
    return getElementInputType(element) === "checkbox" ? element.checked : element.value;
}

function setElementValue(element, value) {
    if (getElementInputType(element) === "checkbox") {
        element.checked = Boolean(value);
        return;
    }

    element.value = value ?? "";
}

function getLastSuccessfulValue(element) {
    let value = element.dataset.lastSuccessfulValue;

    if (getElementInputType(element) === "checkbox") {
        return value === "true";
    }

    return value ?? "";
}

function setLastSuccessfulValue(element, value) {
    element.dataset.lastSuccessfulValue = String(value);
}

async function inputConfigImpl(e) {
    let element = e.currentTarget;
    let key = getConfigKey(element);
    let value = getElementValue(element);
    let lastSuccessfulValue = getLastSuccessfulValue(element);

    if (!key) {
        console.warn("Rejected config update: missing data-config-key");
        setElementValue(element, lastSuccessfulValue);
        return;
    }

    if (String(value) === String(lastSuccessfulValue)) {
        return;
    }

    if (element.hasAttribute("data-path-validate") && !isValidPathValue(value)) {
        console.warn(`Rejected config value for ${key}: invalid path`);
        setElementValue(element, lastSuccessfulValue);
        return;
    }

    console.debug(`Config value for ${key} changed to ${value}`);

    try {
        await invoke("update_config_field", {key, value});
        setLastSuccessfulValue(element, value);
    } catch (error) {
        setElementValue(element, lastSuccessfulValue);
        console.warn(`Reverting ${key} because config save failed: ${error}`);
        return;
    }

    if (key === "language") {
        try {
            await setLocale(value);
        } catch (error) {
            console.error(`Failed to apply locale ${value}.`, error);
        }
    }
}

function getActiveInstanceId() {
    return typeof ACTIVE_INSTANCE_ID === "undefined" ? "saykocraft-earth" : ACTIVE_INSTANCE_ID;
}

async function setModpackSettings() {
    try {
        let instanceSettings = await invoke("get_instance_settings", {id: getActiveInstanceId()});
        applyInstanceSettings(instanceSettings);
    } catch (err) {
        console.error("Failed to load instance settings.", err);
        if (modpackLocationInput) {
            modpackLocationInput.value = t("filesize.unavailable");
        }
    }
}

function getInstanceConfigKey(element) {
    return element.dataset.instanceConfigKey;
}

function getInstanceConfigValue(element) {
    let key = getInstanceConfigKey(element);

    switch (key) {
        case "maximum_ram_mb": {
            return Number.parseInt(element.value, 10);
        }
        case "additional_jvm_args": {
            return parseJvmArguments(element.value);
        }
        default: {
            return element.value;
        }
    }
}

function setInstanceConfigValue(element, value) {
    let key = getInstanceConfigKey(element);

    switch (key) {
        case "maximum_ram_mb": {
            element.value = value ?? "";
            break;
        }
        case "additional_jvm_args": {
            element.value = Array.isArray(value) ? value.join("\n") : "";
            break;
        }
        default: {
            element.value = value ?? "";
        }
    }
}

function serializeInstanceConfigValue(value) {
    return JSON.stringify(value);
}

function deserializeInstanceConfigValue(value, fallback) {
    if (value == null) {
        return fallback;
    }

    try {
        return JSON.parse(value);
    } catch {
        return fallback;
    }
}

function setLastSuccessfulInstanceConfigValue(element, value) {
    element.dataset.lastSuccessfulValue = serializeInstanceConfigValue(value);
}

function getLastSuccessfulInstanceConfigValue(element) {
    return deserializeInstanceConfigValue(
        element.dataset.lastSuccessfulValue,
        getInstanceConfigKey(element) === "additional_jvm_args" ? [] : ""
    );
}

function parseJvmArguments(value) {
    let args = [];
    let current = "";
    let quote = null;

    for (let character of value) {
        if (quote) {
            if (character === quote) {
                quote = null;
            } else {
                current += character;
            }
            continue;
        }

        if (character === "\"" || character === "'") {
            quote = character;
            continue;
        }

        if (/\s/u.test(character)) {
            if (current) {
                args.push(current);
                current = "";
            }
            continue;
        }

        current += character;
    }

    if (current) {
        args.push(current);
    }

    return args;
}

function applyInstanceSettings(payload) {
    let settings = payload.settings ?? {};

    if (modpackLocationInput) {
        modpackLocationInput.value = payload.instance_location ?? "";
    }

    instanceConfigInputs.forEach(element => {
        let key = getInstanceConfigKey(element);

        if (!Object.prototype.hasOwnProperty.call(settings, key)) {
            return;
        }

        if (key === "maximum_ram_mb") {
            element.min = payload.minimum_ram_mb ?? 1;
            element.placeholder = payload.recommended_ram_mb ?? "";
        }

        setInstanceConfigValue(element, settings[key]);
        setLastSuccessfulInstanceConfigValue(element, settings[key]);
    });
}

function registerInstanceConfigListeners() {
    instanceConfigInputs.forEach(element => {
        element.addEventListener("change", inputInstanceConfigImpl);
    });
}

async function inputInstanceConfigImpl(event) {
    let element = event.currentTarget;
    let key = getInstanceConfigKey(element);
    let value = getInstanceConfigValue(element);
    let lastSuccessfulValue = getLastSuccessfulInstanceConfigValue(element);

    if (!key) {
        setInstanceConfigValue(element, lastSuccessfulValue);
        return;
    }

    if (key === "maximum_ram_mb" && !isValidMaximumRamValue(element, value)) {
        setInstanceConfigValue(element, lastSuccessfulValue);
        return;
    }

    if (key === "additional_jvm_args" && value.some(argument => CONTROL_CHARACTER_REGEX.test(argument))) {
        setInstanceConfigValue(element, lastSuccessfulValue);
        return;
    }

    if (serializeInstanceConfigValue(value) === serializeInstanceConfigValue(lastSuccessfulValue)) {
        return;
    }

    try {
        let settings = await invoke("update_instance_settings_field", {
            id: getActiveInstanceId(),
            key,
            value
        });
        setLastSuccessfulInstanceConfigValue(element, settings[key]);
        setInstanceConfigValue(element, settings[key]);
    } catch (err) {
        console.warn(`Reverting instance setting ${key} because save failed: ${err}`);
        setInstanceConfigValue(element, lastSuccessfulValue);
    }
}

function isValidMaximumRamValue(element, value) {
    let minimumValue = Number.parseInt(element.min, 10) || 1;
    return Number.isSafeInteger(value) && value >= minimumValue;
}

registerConfigListeners();
registerInstanceConfigListeners();
initializeLocalization().then(setInitialConfigValues);
