const configInputs = document.querySelectorAll("select[data-config-key], input[data-config-key]");
const CONFIG_PATH_VARIABLE = "$SAYKOCRAFT";
const ABSOLUTE_PATH_REGEX = /^(?:[A-Za-z]:[\\/]|[\\/])/u;
const INVALID_PATH_CHARACTER_REGEX = /[\0-\x1F<>:"|?*]/u;

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

registerConfigListeners();
initializeLocalization().then(setInitialConfigValues);
