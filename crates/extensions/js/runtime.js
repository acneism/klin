(function (config, entries) {
  "use strict";
  const href = String(location.href).split("#")[0];
  const extensionPage = location.protocol === "moz-extension:" && location.host === config.uuid;
  let runs = [];
  if (!extensionPage) {
    if (window !== window.top) {
      return;
    }
    const test = (list) => list.some((source) => new RegExp(source).test(href));
    runs = config.scripts
      .map((script, i) => [script, entries[i]])
      .filter(
        ([s]) =>
          test(s.matches) &&
          !test(s.excludeMatches) &&
          (s.includeGlobs.length === 0 || test(s.includeGlobs)) &&
          !test(s.excludeGlobs),
      );
    if (runs.length === 0) {
      return;
    }
  }

  const debug = console.debug.bind(console);
  const errorLog = console.error.bind(console);
  const stringify = JSON.stringify;
  const defineProperty = Object.defineProperty;
  const freeze = Object.freeze;
  const PromiseCtor = Promise;
  const MARK = "\u0001klin-ext\u0001";
  const random = () => {
    const bytes = new Uint8Array(16);
    crypto.getRandomValues(bytes);
    return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
  };
  const token = random();
  const entry = "__klin_" + random();
  const pending = new Map();
  const listeners = new Map();
  let nextId = 1;

  const send = (message) => {
    message.ext = config.uuid;
    message.token = token;
    debug(MARK + stringify(message));
  };

  const call = (api, args) =>
    new PromiseCtor((resolve, reject) => {
      const id = nextId++;
      pending.set(id, { resolve, reject });
      send({ type: "call", id, api, args });
    });

  const fire = (name, args) => {
    const set = listeners.get(name);
    if (!set) {
      return [];
    }
    const results = [];
    for (const listener of [...set]) {
      try {
        results.push(listener(...args));
      } catch (error) {
        errorLog(error);
      }
    }
    return results;
  };

  const onMessage = (event, args, callId) => {
    let responded = false;
    let waiting = false;
    const respond = (payload) => {
      if (!responded) {
        responded = true;
        send(Object.assign({ type: "respond", callId }, payload));
      }
    };
    const set = listeners.get(event);
    for (const listener of set ? [...set] : []) {
      let result;
      try {
        result = listener(args[0], args[1], (value) => respond({ value }));
      } catch (error) {
        errorLog(error);
        continue;
      }
      if (result === true) {
        waiting = true;
      } else if (result && typeof result.then === "function") {
        waiting = true;
        result.then(
          (value) => respond({ value }),
          (error) => respond({ error: String((error && error.message) || error) }),
        );
      }
    }
    if (!waiting) {
      respond({ none: true });
    }
  };

  const fireSingle = (event, args, callId) => {
    const set = listeners.get(event);
    const listener = set && [...set][0];
    const respond = (value) =>
      send({ type: "respond", callId, value: value === undefined ? null : value });
    if (!listener) {
      respond(null);
      return;
    }
    let result;
    try {
      result = listener(...args);
    } catch (error) {
      errorLog(error);
      respond(null);
      return;
    }
    if (result && typeof result.then === "function") {
      result.then(respond, (error) => {
        errorLog(error);
        respond(null);
      });
    } else {
      respond(result);
    }
  };

  const receive = (message) => {
    if (message.type === "reply") {
      const request = pending.get(message.id);
      if (request) {
        pending.delete(message.id);
        if ("error" in message) {
          request.reject(new Error(message.error));
        } else {
          request.resolve(message.result);
        }
      }
    } else if (message.type === "event") {
      if (message.single && message.callId !== undefined) {
        fireSingle(message.event, message.args, message.callId);
      } else if (message.callId !== undefined) {
        onMessage(message.event, message.args, message.callId);
      } else {
        fire(message.event, message.args);
      }
    }
  };

  defineProperty(window, entry, {
    value: (secret, message) => {
      if (secret === token) {
        receive(message);
      }
    },
    writable: false,
    configurable: false,
    enumerable: false,
  });
  send({ type: "hello", entry, context: extensionPage ? "page" : "content", url: href });

  const event = (name) =>
    freeze({
      addListener(listener) {
        if (!listeners.has(name)) {
          listeners.set(name, new Set());
        }
        listeners.get(name).add(listener);
      },
      removeListener(listener) {
        const set = listeners.get(name);
        if (set) {
          set.delete(listener);
        }
      },
      hasListener(listener) {
        const set = listeners.get(name);
        return !!set && set.has(listener);
      },
      hasListeners() {
        const set = listeners.get(name);
        return !!set && set.size > 0;
      },
    });

  const api = (name) => (...args) => call(name, args);
  const getURL = (path) => "moz-extension://" + config.uuid + "/" + String(path || "").replace(/^\//, "");
  const getMessage = (name, substitutions) => {
    const key = String(name).toLowerCase();
    if (key === "@@extension_id") return config.uuid;
    if (key === "@@ui_locale") return config.uiLanguage.replace("-", "_");
    if (key === "@@bidi_dir") return "ltr";
    let text = config.messages[key];
    if (text === undefined) {
      return "";
    }
    const subs = substitutions === undefined ? [] : [].concat(substitutions).map(String);
    text = text.replace(/\$([a-z0-9_@]+)\$/gi, (whole, placeholder) => {
      const value = config.placeholders[key] && config.placeholders[key][placeholder.toLowerCase()];
      return value === undefined ? whole : value;
    });
    return text.replace(/\$(\d)/g, (whole, digit) => subs[digit - 1] ?? "").replace(/\$\$/g, "$");
  };
  const sendMessageArgs = (args) => {
    if (args.length > 1 && (typeof args[0] === "string" || args[0] === null) && args.length >= 2) {
      return [args[1]];
    }
    return [args[0]];
  };
  const storageArea = (area) =>
    freeze({
      get: (keys) => call("storage.get", [area, keys === undefined ? null : keys]),
      set: (items) => call("storage.set", [area, items]),
      remove: (keys) => call("storage.remove", [area, keys]),
      clear: () => call("storage.clear", [area]),
      onChanged: event("storage." + area + ".onChanged"),
    });

  const browser = {
    runtime: {
      id: config.id,
      getURL,
      getManifest: () => JSON.parse(stringify(config.manifest)),
      sendMessage: (...args) => call("runtime.sendMessage", sendMessageArgs(args)),
      onMessage: event("runtime.onMessage"),
      getPlatformInfo: () => PromiseCtor.resolve({ os: "win", arch: "x86-64" }),
      getBrowserInfo: () =>
        PromiseCtor.resolve({ name: "Klin", vendor: "Klin", version: config.browserVersion, buildID: "" }),
      lastError: null,
    },
    storage: {
      local: storageArea("local"),
      sync: storageArea("sync"),
      session: storageArea("session"),
      onChanged: event("storage.onChanged"),
    },
    i18n: {
      getMessage,
      getUILanguage: () => config.uiLanguage,
      getAcceptLanguages: () => PromiseCtor.resolve([config.uiLanguage, "en-US", "en"]),
    },
    extension: {
      getURL,
      inIncognitoContext: false,
    },
  };

  const permissions = (config.manifest.permissions || []).concat(config.manifest.host_permissions || []);
  const has = (permission) => permissions.includes(permission);
  const webRequestEvent = (name) => {
    const ids = new Map();
    return freeze({
      addListener(listener, filter, extraInfoSpec) {
        if (ids.has(listener)) {
          return;
        }
        const id = random();
        ids.set(listener, id);
        listeners.set("webRequest:" + id, new Set([listener]));
        call("webRequest.addListener", [name, id, filter || {}, extraInfoSpec || []]).catch(errorLog);
      },
      removeListener(listener) {
        const id = ids.get(listener);
        if (id) {
          ids.delete(listener);
          listeners.delete("webRequest:" + id);
          call("webRequest.removeListener", [name, id]).catch(errorLog);
        }
      },
      hasListener(listener) {
        return ids.has(listener);
      },
    });
  };

  if (extensionPage) {
    Object.assign(browser.runtime, {
      onInstalled: event("runtime.onInstalled"),
      onStartup: event("runtime.onStartup"),
      openOptionsPage: api("runtime.openOptionsPage"),
      getBackgroundPage: () => PromiseCtor.resolve(null),
      reload: api("runtime.reload"),
    });
    browser.tabs = {
      query: api("tabs.query"),
      get: api("tabs.get"),
      getCurrent: api("tabs.getCurrent"),
      create: api("tabs.create"),
      update: (...args) => call("tabs.update", typeof args[0] === "object" ? [null, args[0]] : args),
      remove: api("tabs.remove"),
      reload: api("tabs.reload"),
      sendMessage: (tabId, message) => call("tabs.sendMessage", [tabId, message]),
      onCreated: event("tabs.onCreated"),
      onUpdated: event("tabs.onUpdated"),
      onActivated: event("tabs.onActivated"),
      onRemoved: event("tabs.onRemoved"),
      TAB_ID_NONE: -1,
    };
    const action = {
      setBadgeText: api("action.setBadgeText"),
      getBadgeText: api("action.getBadgeText"),
      setBadgeBackgroundColor: api("action.setBadgeBackgroundColor"),
      setTitle: api("action.setTitle"),
      getTitle: api("action.getTitle"),
      setIcon: () => PromiseCtor.resolve(),
      setPopup: api("action.setPopup"),
      enable: api("action.enable"),
      disable: api("action.disable"),
      onClicked: event("action.onClicked"),
    };
    if (config.manifestVersion >= 3) {
      browser.action = action;
    } else {
      browser.browserAction = action;
    }
    if (has("webRequest")) {
      browser.webRequest = {
        onBeforeRequest: webRequestEvent("onBeforeRequest"),
        onBeforeSendHeaders: webRequestEvent("onBeforeSendHeaders"),
        onSendHeaders: webRequestEvent("onSendHeaders"),
        onHeadersReceived: webRequestEvent("onHeadersReceived"),
        onAuthRequired: webRequestEvent("onAuthRequired"),
        onResponseStarted: webRequestEvent("onResponseStarted"),
        onBeforeRedirect: webRequestEvent("onBeforeRedirect"),
        onCompleted: webRequestEvent("onCompleted"),
        onErrorOccurred: webRequestEvent("onErrorOccurred"),
        handlerBehaviorChanged: api("webRequest.handlerBehaviorChanged"),
        MAX_HANDLER_BEHAVIOR_CHANGED_CALLS_PER_10_MINUTES: 20,
        ResourceType: freeze({
          MAIN_FRAME: "main_frame", SUB_FRAME: "sub_frame", STYLESHEET: "stylesheet", SCRIPT: "script",
          IMAGE: "image", OBJECT: "object", OBJECT_SUBREQUEST: "object_subrequest",
          XMLHTTPREQUEST: "xmlhttprequest", XSLT: "xslt", PING: "ping", BEACON: "beacon",
          XML_DTD: "xml_dtd", FONT: "font", MEDIA: "media", WEBSOCKET: "websocket",
          CSP_REPORT: "csp_report", IMAGESET: "imageset", WEB_MANIFEST: "web_manifest",
          SPECULATIVE: "speculative", OTHER: "other",
        }),
      };
    }
    if (has("declarativeNetRequest") || has("declarativeNetRequestWithHostAccess")) {
      browser.declarativeNetRequest = {
        updateDynamicRules: (options) => call("declarativeNetRequest.updateDynamicRules", [options || {}]),
        getDynamicRules: (filter) => call("declarativeNetRequest.getDynamicRules", [filter || {}]),
        updateSessionRules: (options) => call("declarativeNetRequest.updateSessionRules", [options || {}]),
        getSessionRules: (filter) => call("declarativeNetRequest.getSessionRules", [filter || {}]),
        getEnabledRulesets: () => call("declarativeNetRequest.getEnabledRulesets", [{}]),
        updateEnabledRulesets: (options) => call("declarativeNetRequest.updateEnabledRulesets", [options || {}]),
        isRegexSupported: (options) => call("declarativeNetRequest.isRegexSupported", [options || {}]),
        getAvailableStaticRuleCount: () => call("declarativeNetRequest.getAvailableStaticRuleCount", [{}]),
        MAX_NUMBER_OF_DYNAMIC_RULES: 30000,
        MAX_NUMBER_OF_SESSION_RULES: 30000,
        MAX_NUMBER_OF_REGEX_RULES: 1000,
        MAX_NUMBER_OF_ENABLED_STATIC_RULESETS: 50,
        GUARANTEED_MINIMUM_STATIC_RULES: 30000,
        DYNAMIC_RULESET_ID: "_dynamic",
        SESSION_RULESET_ID: "_session",
      };
    }
  }

  const callbackStyle = (target) => {
    const out = {};
    for (const key of Object.keys(target)) {
      const value = target[key];
      if (typeof value === "function" && !/^[A-Z]/.test(key)) {
        out[key] = (...args) => {
          const callback = typeof args[args.length - 1] === "function" ? args.pop() : null;
          const result = value(...args);
          if (!callback || !result || typeof result.then !== "function") {
            return result;
          }
          result.then(
            (value) => callback(value),
            (error) => {
              chrome.runtime.lastError = { message: String((error && error.message) || error) };
              try {
                callback();
              } finally {
                chrome.runtime.lastError = undefined;
              }
            },
          );
          return undefined;
        };
      } else if (value && typeof value === "object" && !("addListener" in value)) {
        out[key] = callbackStyle(value);
      } else {
        out[key] = value;
      }
    }
    return out;
  };
  const chrome = callbackStyle(browser);
  chrome.runtime.lastError = undefined;

  if (extensionPage) {
    defineProperty(window, "browser", { value: browser, configurable: true });
    defineProperty(window, "chrome", { value: chrome, configurable: true });
    return;
  }

  const runEntry = ([script, code]) => {
    if (script.css.length) {
      const style = document.createElement("style");
      style.textContent = script.css.join("\n");
      (document.head || document.documentElement).appendChild(style);
    }
    try {
      code.call(window, browser, chrome);
    } catch (error) {
      errorLog(error);
    }
  };
  for (const run of runs) {
    const runAt = run[0].runAt;
    if (runAt === "document_start") {
      runEntry(run);
    } else if (runAt === "document_end" && document.readyState !== "loading") {
      runEntry(run);
    } else if (runAt === "document_idle" && document.readyState === "complete") {
      runEntry(run);
    } else {
      document.addEventListener(
        "DOMContentLoaded",
        () => (runAt === "document_end" ? runEntry(run) : setTimeout(() => runEntry(run), 0)),
        { once: true },
      );
    }
  }
})
