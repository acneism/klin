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
    const inject = config.inject.length > 0 && test(config.inject);
    if (runs.length === 0 && !inject) {
      return;
    }
  }

  if (typeof HTMLDocument === "function" && !(document instanceof HTMLDocument) && document.documentElement instanceof HTMLHtmlElement) {
    try {
      Object.defineProperty(HTMLDocument, Symbol.hasInstance, {
        value: (value) =>
          value instanceof Document &&
          !!value.documentElement &&
          value.documentElement.namespaceURI === "http://www.w3.org/1999/xhtml" &&
          value.contentType !== "application/xhtml+xml",
        configurable: true,
      });
    } catch {}
  }

  if (self.crypto && typeof self.crypto.randomUUID !== "function" && typeof self.crypto.getRandomValues === "function") {
    try {
      Object.defineProperty(Object.getPrototypeOf(self.crypto), "randomUUID", {
        value: function randomUUID() {
          const b = self.crypto.getRandomValues(new Uint8Array(16));
          b[6] = (b[6] & 0x0f) | 0x40;
          b[8] = (b[8] & 0x3f) | 0x80;
          const h = Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
          return h.slice(0, 8) + "-" + h.slice(8, 12) + "-" + h.slice(12, 16) + "-" + h.slice(16, 20) + "-" + h.slice(20);
        },
        configurable: true,
        writable: true,
      });
    } catch {}
  }

  if (typeof self.requestIdleCallback !== "function") {
    const idle = new Map();
    let nextIdle = 1;
    self.requestIdleCallback = (callback, options) => {
      const id = nextIdle++;
      const start = Date.now();
      const delay = options && options.timeout ? Math.min(options.timeout, 50) : 1;
      idle.set(id, setTimeout(() => {
        idle.delete(id);
        callback({ didTimeout: false, timeRemaining: () => Math.max(0, 50 - (Date.now() - start)) });
      }, delay));
      return id;
    };
    self.cancelIdleCallback = (id) => {
      clearTimeout(idle.get(id));
      idle.delete(id);
    };
  }

  if (!extensionPage && !("wrappedJSObject" in window)) {
    try {
      Object.defineProperty(window, "wrappedJSObject", { value: window, configurable: true });
      Object.defineProperty(window, "cloneInto", { value: (value) => value, configurable: true });
      Object.defineProperty(window, "exportFunction", {
        value: (fn, target, options) => {
          if (options && options.defineAs) {
            target[options.defineAs] = fn;
          }
          return fn;
        },
        configurable: true,
      });
    } catch {}
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

  const ports = new Map();
  const makePort = (id, name, sender) => {
    const portListeners = { message: new Set(), disconnect: new Set() };
    const portEvent = (kind) =>
      freeze({
        addListener: (listener) => portListeners[kind].add(listener),
        removeListener: (listener) => portListeners[kind].delete(listener),
        hasListener: (listener) => portListeners[kind].has(listener),
      });
    let connected = true;
    const port = {
      name: name || "",
      sender,
      error: null,
      onMessage: portEvent("message"),
      onDisconnect: portEvent("disconnect"),
      postMessage(message) {
        if (!connected) {
          throw new Error("Attempt to postMessage on disconnected port");
        }
        send({ type: "port", action: "post", port: id, message });
      },
      disconnect() {
        if (connected) {
          connected = false;
          ports.delete(id);
          send({ type: "port", action: "disconnect", port: id });
        }
      },
    };
    ports.set(id, {
      port,
      deliver(message) {
        for (const listener of [...portListeners.message]) {
          try {
            listener(message, port);
          } catch (error) {
            errorLog(error);
          }
        }
      },
      close(error) {
        if (!connected) {
          return;
        }
        connected = false;
        ports.delete(id);
        port.error = error ? { message: error } : null;
        for (const listener of [...portListeners.disconnect]) {
          try {
            listener(port);
          } catch (e) {
            errorLog(e);
          }
        }
      },
    });
    return port;
  };
  const connect = (tabId, info) => {
    const id = random();
    const name = info && info.name;
    const port = makePort(id, name, undefined);
    send({ type: "port", action: "connect", port: id, name: name || "", tabId });
    return port;
  };
  const styleSheets = new Map();

  const handle = (secret, message) => {
      if (secret !== token) {
        return undefined;
      }
      if (message.type === "api") {
        return { browser, chrome };
      }
      if (message.type === "css") {
        const key = message.key;
        if (message.remove) {
          const sheet = styleSheets.get(key);
          if (sheet) {
            styleSheets.delete(key);
            if (sheet.node) {
              sheet.node.remove();
            } else {
              document.adoptedStyleSheets = document.adoptedStyleSheets.filter((s) => s !== sheet.sheet);
            }
          }
          return true;
        }
        if (styleSheets.has(key)) {
          return true;
        }
        try {
          const sheet = new CSSStyleSheet();
          sheet.replaceSync(message.css);
          document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
          styleSheets.set(key, { sheet });
        } catch {
          const node = document.createElement("style");
          node.textContent = message.css;
          (document.head || document.documentElement).appendChild(node);
          styleSheets.set(key, { node });
        }
        return true;
      }
      if (message.type === "port") {
        const entryPort = ports.get(message.port);
        if (message.action === "connect") {
          const port = makePort(message.port, message.name, message.sender);
          fire("runtime.onConnect", [port]);
        } else if (entryPort && message.action === "message") {
          entryPort.deliver(message.message);
        } else if (entryPort && message.action === "disconnect") {
          entryPort.close(message.error);
        }
        return undefined;
      }
      receive(message);
      return undefined;
  };
  defineProperty(window, entry, {
    value: (secret, message) => {
      try {
        return handle(secret, message);
      } catch (error) {
        errorLog("klin: " + (error && error.stack ? error.stack : String(error)));
        return undefined;
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
      connect: (extensionId, info) => connect(null, typeof extensionId === "object" ? extensionId : info),
      onConnect: event("runtime.onConnect"),
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
      isAllowedIncognitoAccess: () => PromiseCtor.resolve(false),
    },
    dom: {
      openOrClosedShadowRoot: (element) => element.openOrClosedShadowRoot || element.shadowRoot || null,
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

  const alarms = new Map();
  const alarmApi = {
    create(name, info) {
      if (typeof name === "object") {
        info = name;
        name = "";
      }
      name = name || "";
      info = info || {};
      const old = alarms.get(name);
      if (old) {
        clearTimeout(old.timer);
      }
      const period = info.periodInMinutes;
      const when = info.when !== undefined ? info.when : Date.now() + (info.delayInMinutes !== undefined ? info.delayInMinutes : period || 0) * 60000;
      const alarm = { name, scheduledTime: when, periodInMinutes: period };
      const schedule = () => {
        alarm.timer = setTimeout(() => {
          fire("alarms.onAlarm", [{ name, scheduledTime: alarm.scheduledTime, periodInMinutes: period }]);
          if (period) {
            alarm.scheduledTime = Date.now() + period * 60000;
            schedule();
          } else {
            alarms.delete(name);
          }
        }, Math.max(0, alarm.scheduledTime - Date.now()));
      };
      alarms.set(name, alarm);
      schedule();
      return PromiseCtor.resolve();
    },
    get: (name) => {
      const alarm = alarms.get(name || "");
      return PromiseCtor.resolve(alarm ? { name: alarm.name, scheduledTime: alarm.scheduledTime, periodInMinutes: alarm.periodInMinutes } : undefined);
    },
    getAll: () => PromiseCtor.resolve([...alarms.values()].map((a) => ({ name: a.name, scheduledTime: a.scheduledTime, periodInMinutes: a.periodInMinutes }))),
    clear(name) {
      const alarm = alarms.get(name || "");
      if (alarm) {
        clearTimeout(alarm.timer);
        alarms.delete(name || "");
      }
      return PromiseCtor.resolve(!!alarm);
    },
    clearAll() {
      for (const alarm of alarms.values()) {
        clearTimeout(alarm.timer);
      }
      const had = alarms.size > 0;
      alarms.clear();
      return PromiseCtor.resolve(had);
    },
    onAlarm: event("alarms.onAlarm"),
  };
  let nextMenu = 1;
  const menusApi = {
    create(properties, callback) {
      const id = (properties && properties.id) || nextMenu++;
      if (typeof callback === "function") {
        setTimeout(callback, 0);
      }
      return id;
    },
    update: () => PromiseCtor.resolve(),
    remove: () => PromiseCtor.resolve(),
    removeAll: () => PromiseCtor.resolve(),
    refresh: () => PromiseCtor.resolve(),
    onClicked: event("menus.onClicked"),
    onShown: event("menus.onShown"),
    onHidden: event("menus.onHidden"),
    ContextType: freeze({ ALL: "all", PAGE: "page", FRAME: "frame", SELECTION: "selection", LINK: "link", EDITABLE: "editable", IMAGE: "image", VIDEO: "video", AUDIO: "audio", TAB: "tab", BROWSER_ACTION: "browser_action", PAGE_ACTION: "page_action" }),
  };
  const navigationEvent = (name) => {
    const ids = new Map();
    return freeze({
      addListener(listener) {
        if (!ids.has(listener)) {
          ids.set(listener, true);
          event("webNavigation." + name).addListener(listener);
        }
      },
      removeListener(listener) {
        ids.delete(listener);
        event("webNavigation." + name).removeListener(listener);
      },
      hasListener: (listener) => ids.has(listener),
    });
  };

  if (extensionPage) {
    Object.assign(browser.runtime, {
      onInstalled: event("runtime.onInstalled"),
      onStartup: event("runtime.onStartup"),
      openOptionsPage: api("runtime.openOptionsPage"),
      getBackgroundPage: () => PromiseCtor.resolve(null),
      reload: api("runtime.reload"),
      onUpdateAvailable: event("runtime.onUpdateAvailable"),
      onSuspend: event("runtime.onSuspend"),
      setUninstallURL: () => PromiseCtor.resolve(),
    });
    browser.storage.managed = freeze({
      get: () => PromiseCtor.resolve({}),
      onChanged: event("storage.managed.onChanged"),
    });
    if (has("alarms")) {
      browser.alarms = alarmApi;
    }
    if (has("menus") || has("contextMenus")) {
      browser.menus = menusApi;
      browser.contextMenus = menusApi;
    }
    browser.windows = {
      get: (windowId, info) => call("windows.get", [windowId, info || {}]),
      getCurrent: (info) => call("windows.get", [-2, info || {}]),
      getLastFocused: (info) => call("windows.get", [-2, info || {}]),
      getAll: (info) => call("windows.getAll", [info || {}]),
      create: (data) => call("windows.create", [data || {}]),
      update: (windowId, info) => call("windows.update", [windowId, info || {}]),
      remove: (windowId) => call("windows.remove", [windowId]),
      onCreated: event("windows.onCreated"),
      onRemoved: event("windows.onRemoved"),
      onFocusChanged: event("windows.onFocusChanged"),
      WINDOW_ID_NONE: -1,
      WINDOW_ID_CURRENT: -2,
      WindowType: freeze({ NORMAL: "normal", POPUP: "popup", PANEL: "panel", DEVTOOLS: "devtools" }),
      WindowState: freeze({ NORMAL: "normal", MINIMIZED: "minimized", MAXIMIZED: "maximized", FULLSCREEN: "fullscreen", DOCKED: "docked" }),
    };
    if (has("webNavigation")) {
      browser.webNavigation = {
        onBeforeNavigate: navigationEvent("onBeforeNavigate"),
        onCommitted: navigationEvent("onCommitted"),
        onDOMContentLoaded: navigationEvent("onDOMContentLoaded"),
        onCompleted: navigationEvent("onCompleted"),
        onErrorOccurred: navigationEvent("onErrorOccurred"),
        onCreatedNavigationTarget: navigationEvent("onCreatedNavigationTarget"),
        onReferenceFragmentUpdated: navigationEvent("onReferenceFragmentUpdated"),
        onHistoryStateUpdated: navigationEvent("onHistoryStateUpdated"),
        onTabReplaced: navigationEvent("onTabReplaced"),
        getFrame: (details) => call("webNavigation.getFrame", [details || {}]),
        getAllFrames: (details) => call("webNavigation.getAllFrames", [details || {}]),
      };
    }
    browser.contentScripts = {
      register: (options) =>
        call("contentScripts.register", [options || {}]).then((id) => ({
          unregister: () => call("contentScripts.unregister", [id]),
        })),
    };
    browser.tabs = {
      query: api("tabs.query"),
      get: api("tabs.get"),
      getCurrent: api("tabs.getCurrent"),
      create: api("tabs.create"),
      update: (...args) => call("tabs.update", typeof args[0] === "object" ? [null, args[0]] : args),
      remove: api("tabs.remove"),
      reload: api("tabs.reload"),
      sendMessage: (tabId, message) => call("tabs.sendMessage", [tabId, message]),
      connect: (tabId, info) => connect(tabId, info),
      executeScript: (tabId, details) =>
        typeof tabId === "object" && tabId !== null
          ? call("tabs.executeScript", [null, tabId])
          : call("tabs.executeScript", [tabId === undefined ? null : tabId, details || {}]),
      insertCSS: (tabId, details) =>
        typeof tabId === "object" && tabId !== null
          ? call("tabs.insertCSS", [null, tabId])
          : call("tabs.insertCSS", [tabId === undefined ? null : tabId, details || {}]),
      removeCSS: (tabId, details) =>
        typeof tabId === "object" && tabId !== null
          ? call("tabs.removeCSS", [null, tabId])
          : call("tabs.removeCSS", [tabId === undefined ? null : tabId, details || {}]),
      move: api("tabs.move"),
      duplicate: api("tabs.duplicate"),
      discard: () => PromiseCtor.resolve(),
      highlight: () => PromiseCtor.resolve(),
      getZoom: () => PromiseCtor.resolve(1),
      onReplaced: event("tabs.onReplaced"),
      onMoved: event("tabs.onMoved"),
      onAttached: event("tabs.onAttached"),
      onDetached: event("tabs.onDetached"),
      onHighlighted: event("tabs.onHighlighted"),
      onZoomChange: event("tabs.onZoomChange"),
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
      setBadgeTextColor: () => PromiseCtor.resolve(),
      getBadgeBackgroundColor: () => PromiseCtor.resolve([217, 0, 0, 255]),
      isEnabled: () => PromiseCtor.resolve(true),
      getPopup: api("action.getPopup"),
      openPopup: () => PromiseCtor.resolve(),
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
