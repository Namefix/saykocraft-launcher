const DEFAULT_LOCALE = "en-US";
const LOCALE_FILES = {
    "en-US": "locales/en-US.json",
    "tr-TR": "locales/tr-TR.json"
};

let currentLocale = DEFAULT_LOCALE;
let currentMessages = {};
let fallbackMessages = {};
let localeLoadPromise = null;

function resolveLocale(locale) {
    return LOCALE_FILES[locale] ? locale : DEFAULT_LOCALE;
}

async function loadLocaleFile(locale) {
    const filePath = LOCALE_FILES[locale] || LOCALE_FILES[DEFAULT_LOCALE];
    const response = await fetch(filePath);

    if (!response.ok) {
        throw new Error(`Failed to load locale file: ${filePath}`);
    }

    return await response.json();
}

function interpolate(template, values) {
    return template.replace(/\{(\w+)\}/g, (_, key) => {
        const value = values?.[key];
        return value == null ? `{${key}}` : String(value);
    });
}

function t(key, values) {
    const template = currentMessages[key] ?? fallbackMessages[key] ?? key;
    return values ? interpolate(template, values) : template;
}

function applyTranslations(root = document) {
    root.querySelectorAll("[data-i18n]").forEach((element) => {
        element.textContent = t(element.dataset.i18n);
    });

    root.querySelectorAll("[data-i18n-placeholder]").forEach((element) => {
        element.setAttribute("placeholder", t(element.dataset.i18nPlaceholder));
    });

    root.querySelectorAll("[data-i18n-title]").forEach((element) => {
        element.setAttribute("title", t(element.dataset.i18nTitle));
    });

    root.querySelectorAll("[data-i18n-aria-label]").forEach((element) => {
        element.setAttribute("aria-label", t(element.dataset.i18nAriaLabel));
    });
}

async function loadTranslations(locale) {
    const resolvedLocale = resolveLocale(locale);
    const [resolvedMessages, resolvedFallbackMessages] = await Promise.all([
        loadLocaleFile(resolvedLocale),
        resolvedLocale === DEFAULT_LOCALE ? Promise.resolve(null) : loadLocaleFile(DEFAULT_LOCALE)
    ]);

    currentMessages = resolvedMessages;
    fallbackMessages = resolvedFallbackMessages || resolvedMessages;
    currentLocale = resolvedLocale;
    document.documentElement.lang = resolvedLocale;
    applyTranslations();
}

async function setLocale(locale) {
    await loadTranslations(locale);
}

async function initializeLocalization() {
    if (!localeLoadPromise) {
        localeLoadPromise = (async () => {
            try {
                const config = await get_config();
                await loadTranslations(config.language);
                return config;
            } catch {
                await loadTranslations(document.documentElement.lang || DEFAULT_LOCALE);
                return null;
            }
        })();
    }

    return await localeLoadPromise;
}
