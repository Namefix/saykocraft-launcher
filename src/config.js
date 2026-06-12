const configInputs = document.querySelectorAll('select[id^="config-"], input[id^="config-"]');

function registerConfigListeners() {
    configInputs.forEach(config => {
        switch(config.localName) {
            case "input": {
                config.addEventListener("input", config.getAttribute("type") === "checkbox" ? inputCheckboxImpl : inputDefaultImpl);
                break;
            }
            case "select": {
                config.addEventListener("change", inputSelectImpl);
            }
        }
        
    })
}

async function setInitialConfigValues(configValues) {
    if (!configValues) {
        return;
    }

    await setLocale(configValues.language);

    configInputs.forEach(element => {
        for(let value in configValues) {
            if(element.id.replace("config-", "") === value) {
                switch(getElementInputType(element)) {
                    case "text": 
                    case "select":
                    {
                        element.value = configValues[value];
                        break;
                    }
                    case "checkbox": {
                        element.checked = configValues[value];
                        break;
                    }
                }
            }
        }
    });
}

function getElementInputType(e) {
    if(e.localName === "input") {
        return e.getAttribute("type");
    }
    return e.localName;
}

function inputDefaultImpl(e) {
    let key = e.target.id.replace("config-", "");
    let value = e.target.value;

    console.log(`Config value for ${key} changed to ${value}`);
    invoke("update_config_field", {key, value});
}

function inputCheckboxImpl(e) {
    let key = e.target.id.replace("config-", "");
    let value = e.target.checked;

    console.log(`Config value for ${key} changed to ${value}`);
    invoke("update_config_field", {key, value});
}

async function inputSelectImpl(e) {
    let key = e.target.id.replace("config-", "");
    let value = e.target.value;

    if (key === "language") {
        await setLocale(value);
    }

    console.log(`Config value for ${key} changed to ${value}`);
    invoke("update_config_field", {key, value});
}

registerConfigListeners();
initializeLocalization().then(setInitialConfigValues);