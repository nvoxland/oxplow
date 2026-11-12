(() => {
  var __create = Object.create;
  var __getProtoOf = Object.getPrototypeOf;
  var __defProp = Object.defineProperty;
  var __getOwnPropNames = Object.getOwnPropertyNames;
  var __hasOwnProp = Object.prototype.hasOwnProperty;
  function __accessProp(key) {
    return this[key];
  }
  var __toESMCache_node;
  var __toESMCache_esm;
  var __toESM = (mod, isNodeMode, target) => {
    var canCache = mod != null && typeof mod === "object";
    if (canCache) {
      var cache = isNodeMode ? __toESMCache_node ??= new WeakMap : __toESMCache_esm ??= new WeakMap;
      var cached = cache.get(mod);
      if (cached)
        return cached;
    }
    target = mod != null ? __create(__getProtoOf(mod)) : {};
    const to = isNodeMode || !mod || !mod.__esModule || !__hasOwnProp.call(mod, "default") ? __defProp(target, "default", { value: mod, enumerable: true }) : target;
    if (mod && typeof mod === "object" || typeof mod === "function") {
      for (let key of __getOwnPropNames(mod))
        if (!__hasOwnProp.call(to, key))
          __defProp(to, key, {
            get: __accessProp.bind(mod, key),
            enumerable: true
          });
    }
    if (canCache)
      cache.set(mod, to);
    return to;
  };
  var __commonJS = (cb, mod) => () => (mod || cb((mod = { exports: {} }).exports, mod), mod.exports);
  var __returnValue = (v) => v;
  function __exportSetter(name, newValue) {
    this[name] = __returnValue.bind(null, newValue);
  }
  var __export = (target, all) => {
    for (var name in all)
      __defProp(target, name, {
        get: all[name],
        enumerable: true,
        configurable: true,
        set: __exportSetter.bind(all, name)
      });
  };
  var __require = /* @__PURE__ */ ((x) => typeof require !== "undefined" ? require : typeof Proxy !== "undefined" ? new Proxy(x, {
    get: (a, b) => (typeof require !== "undefined" ? require : a)[b]
  }) : x)(function(x) {
    if (typeof require !== "undefined")
      return require.apply(this, arguments);
    throw Error('Dynamic require of "' + x + '" is not supported');
  });

  // ../../node_modules/.bun/ms@2.1.3/node_modules/ms/index.js
  var require_ms = __commonJS(function(exports, module) {
    var s = 1000;
    var m = s * 60;
    var h = m * 60;
    var d = h * 24;
    var w = d * 7;
    var y = d * 365.25;
    module.exports = function(val, options) {
      options = options || {};
      var type = typeof val;
      if (type === "string" && val.length > 0) {
        return parse(val);
      } else if (type === "number" && isFinite(val)) {
        return options.long ? fmtLong(val) : fmtShort(val);
      }
      throw new Error("val is not a non-empty string or a valid number. val=" + JSON.stringify(val));
    };
    function parse(str) {
      str = String(str);
      if (str.length > 100) {
        return;
      }
      var match = /^(-?(?:\d+)?\.?\d+) *(milliseconds?|msecs?|ms|seconds?|secs?|s|minutes?|mins?|m|hours?|hrs?|h|days?|d|weeks?|w|years?|yrs?|y)?$/i.exec(str);
      if (!match) {
        return;
      }
      var n = parseFloat(match[1]);
      var type = (match[2] || "ms").toLowerCase();
      switch (type) {
        case "years":
        case "year":
        case "yrs":
        case "yr":
        case "y":
          return n * y;
        case "weeks":
        case "week":
        case "w":
          return n * w;
        case "days":
        case "day":
        case "d":
          return n * d;
        case "hours":
        case "hour":
        case "hrs":
        case "hr":
        case "h":
          return n * h;
        case "minutes":
        case "minute":
        case "mins":
        case "min":
        case "m":
          return n * m;
        case "seconds":
        case "second":
        case "secs":
        case "sec":
        case "s":
          return n * s;
        case "milliseconds":
        case "millisecond":
        case "msecs":
        case "msec":
        case "ms":
          return n;
        default:
          return;
      }
    }
    function fmtShort(ms) {
      var msAbs = Math.abs(ms);
      if (msAbs >= d) {
        return Math.round(ms / d) + "d";
      }
      if (msAbs >= h) {
        return Math.round(ms / h) + "h";
      }
      if (msAbs >= m) {
        return Math.round(ms / m) + "m";
      }
      if (msAbs >= s) {
        return Math.round(ms / s) + "s";
      }
      return ms + "ms";
    }
    function fmtLong(ms) {
      var msAbs = Math.abs(ms);
      if (msAbs >= d) {
        return plural(ms, msAbs, d, "day");
      }
      if (msAbs >= h) {
        return plural(ms, msAbs, h, "hour");
      }
      if (msAbs >= m) {
        return plural(ms, msAbs, m, "minute");
      }
      if (msAbs >= s) {
        return plural(ms, msAbs, s, "second");
      }
      return ms + " ms";
    }
    function plural(ms, msAbs, n, name) {
      var isPlural = msAbs >= n * 1.5;
      return Math.round(ms / n) + " " + name + (isPlural ? "s" : "");
    }
  });

  // ../../node_modules/.bun/debug@4.4.3/node_modules/debug/src/common.js
  var require_common = __commonJS(function(exports, module) {
    function setup(env) {
      createDebug.debug = createDebug;
      createDebug.default = createDebug;
      createDebug.coerce = coerce;
      createDebug.disable = disable;
      createDebug.enable = enable;
      createDebug.enabled = enabled;
      createDebug.humanize = require_ms();
      createDebug.destroy = destroy;
      Object.keys(env).forEach((key) => {
        createDebug[key] = env[key];
      });
      createDebug.names = [];
      createDebug.skips = [];
      createDebug.formatters = {};
      function selectColor(namespace) {
        let hash = 0;
        for (let i = 0;i < namespace.length; i++) {
          hash = (hash << 5) - hash + namespace.charCodeAt(i);
          hash |= 0;
        }
        return createDebug.colors[Math.abs(hash) % createDebug.colors.length];
      }
      createDebug.selectColor = selectColor;
      function createDebug(namespace) {
        let prevTime;
        let enableOverride = null;
        let namespacesCache;
        let enabledCache;
        function debug(...args) {
          if (!debug.enabled) {
            return;
          }
          const self2 = debug;
          const curr = Number(new Date);
          const ms = curr - (prevTime || curr);
          self2.diff = ms;
          self2.prev = prevTime;
          self2.curr = curr;
          prevTime = curr;
          args[0] = createDebug.coerce(args[0]);
          if (typeof args[0] !== "string") {
            args.unshift("%O");
          }
          let index = 0;
          args[0] = args[0].replace(/%([a-zA-Z%])/g, (match, format) => {
            if (match === "%%") {
              return "%";
            }
            index++;
            const formatter = createDebug.formatters[format];
            if (typeof formatter === "function") {
              const val = args[index];
              match = formatter.call(self2, val);
              args.splice(index, 1);
              index--;
            }
            return match;
          });
          createDebug.formatArgs.call(self2, args);
          const logFn = self2.log || createDebug.log;
          logFn.apply(self2, args);
        }
        debug.namespace = namespace;
        debug.useColors = createDebug.useColors();
        debug.color = createDebug.selectColor(namespace);
        debug.extend = extend;
        debug.destroy = createDebug.destroy;
        Object.defineProperty(debug, "enabled", {
          enumerable: true,
          configurable: false,
          get: () => {
            if (enableOverride !== null) {
              return enableOverride;
            }
            if (namespacesCache !== createDebug.namespaces) {
              namespacesCache = createDebug.namespaces;
              enabledCache = createDebug.enabled(namespace);
            }
            return enabledCache;
          },
          set: (v) => {
            enableOverride = v;
          }
        });
        if (typeof createDebug.init === "function") {
          createDebug.init(debug);
        }
        return debug;
      }
      function extend(namespace, delimiter) {
        const newDebug = createDebug(this.namespace + (typeof delimiter === "undefined" ? ":" : delimiter) + namespace);
        newDebug.log = this.log;
        return newDebug;
      }
      function enable(namespaces) {
        createDebug.save(namespaces);
        createDebug.namespaces = namespaces;
        createDebug.names = [];
        createDebug.skips = [];
        const split = (typeof namespaces === "string" ? namespaces : "").trim().replace(/\s+/g, ",").split(",").filter(Boolean);
        for (const ns of split) {
          if (ns[0] === "-") {
            createDebug.skips.push(ns.slice(1));
          } else {
            createDebug.names.push(ns);
          }
        }
      }
      function matchesTemplate(search, template) {
        let searchIndex = 0;
        let templateIndex = 0;
        let starIndex = -1;
        let matchIndex = 0;
        while (searchIndex < search.length) {
          if (templateIndex < template.length && (template[templateIndex] === search[searchIndex] || template[templateIndex] === "*")) {
            if (template[templateIndex] === "*") {
              starIndex = templateIndex;
              matchIndex = searchIndex;
              templateIndex++;
            } else {
              searchIndex++;
              templateIndex++;
            }
          } else if (starIndex !== -1) {
            templateIndex = starIndex + 1;
            matchIndex++;
            searchIndex = matchIndex;
          } else {
            return false;
          }
        }
        while (templateIndex < template.length && template[templateIndex] === "*") {
          templateIndex++;
        }
        return templateIndex === template.length;
      }
      function disable() {
        const namespaces = [
          ...createDebug.names,
          ...createDebug.skips.map((namespace) => "-" + namespace)
        ].join(",");
        createDebug.enable("");
        return namespaces;
      }
      function enabled(name) {
        for (const skip of createDebug.skips) {
          if (matchesTemplate(name, skip)) {
            return false;
          }
        }
        for (const ns of createDebug.names) {
          if (matchesTemplate(name, ns)) {
            return true;
          }
        }
        return false;
      }
      function coerce(val) {
        if (val instanceof Error) {
          return val.stack || val.message;
        }
        return val;
      }
      function destroy() {
        console.warn("Instance method `debug.destroy()` is deprecated and no longer does anything. It will be removed in the next major version of `debug`.");
      }
      createDebug.enable(createDebug.load());
      return createDebug;
    }
    module.exports = setup;
  });

  // ../../node_modules/.bun/debug@4.4.3/node_modules/debug/src/browser.js
  var require_browser = __commonJS(function(exports, module) {
    exports.formatArgs = formatArgs;
    exports.save = save;
    exports.load = load;
    exports.useColors = useColors;
    exports.storage = localstorage();
    exports.destroy = (() => {
      let warned = false;
      return () => {
        if (!warned) {
          warned = true;
          console.warn("Instance method `debug.destroy()` is deprecated and no longer does anything. It will be removed in the next major version of `debug`.");
        }
      };
    })();
    exports.colors = [
      "#0000CC",
      "#0000FF",
      "#0033CC",
      "#0033FF",
      "#0066CC",
      "#0066FF",
      "#0099CC",
      "#0099FF",
      "#00CC00",
      "#00CC33",
      "#00CC66",
      "#00CC99",
      "#00CCCC",
      "#00CCFF",
      "#3300CC",
      "#3300FF",
      "#3333CC",
      "#3333FF",
      "#3366CC",
      "#3366FF",
      "#3399CC",
      "#3399FF",
      "#33CC00",
      "#33CC33",
      "#33CC66",
      "#33CC99",
      "#33CCCC",
      "#33CCFF",
      "#6600CC",
      "#6600FF",
      "#6633CC",
      "#6633FF",
      "#66CC00",
      "#66CC33",
      "#9900CC",
      "#9900FF",
      "#9933CC",
      "#9933FF",
      "#99CC00",
      "#99CC33",
      "#CC0000",
      "#CC0033",
      "#CC0066",
      "#CC0099",
      "#CC00CC",
      "#CC00FF",
      "#CC3300",
      "#CC3333",
      "#CC3366",
      "#CC3399",
      "#CC33CC",
      "#CC33FF",
      "#CC6600",
      "#CC6633",
      "#CC9900",
      "#CC9933",
      "#CCCC00",
      "#CCCC33",
      "#FF0000",
      "#FF0033",
      "#FF0066",
      "#FF0099",
      "#FF00CC",
      "#FF00FF",
      "#FF3300",
      "#FF3333",
      "#FF3366",
      "#FF3399",
      "#FF33CC",
      "#FF33FF",
      "#FF6600",
      "#FF6633",
      "#FF9900",
      "#FF9933",
      "#FFCC00",
      "#FFCC33"
    ];
    function useColors() {
      if (typeof window !== "undefined" && window.process && (window.process.type === "renderer" || window.process.__nwjs)) {
        return true;
      }
      if (typeof navigator !== "undefined" && navigator.userAgent && navigator.userAgent.toLowerCase().match(/(edge|trident)\/(\d+)/)) {
        return false;
      }
      let m;
      return typeof document !== "undefined" && document.documentElement && document.documentElement.style && document.documentElement.style.WebkitAppearance || typeof window !== "undefined" && window.console && (window.console.firebug || window.console.exception && window.console.table) || typeof navigator !== "undefined" && navigator.userAgent && (m = navigator.userAgent.toLowerCase().match(/firefox\/(\d+)/)) && parseInt(m[1], 10) >= 31 || typeof navigator !== "undefined" && navigator.userAgent && navigator.userAgent.toLowerCase().match(/applewebkit\/(\d+)/);
    }
    function formatArgs(args) {
      args[0] = (this.useColors ? "%c" : "") + this.namespace + (this.useColors ? " %c" : " ") + args[0] + (this.useColors ? "%c " : " ") + "+" + module.exports.humanize(this.diff);
      if (!this.useColors) {
        return;
      }
      const c = "color: " + this.color;
      args.splice(1, 0, c, "color: inherit");
      let index = 0;
      let lastC = 0;
      args[0].replace(/%[a-zA-Z%]/g, (match) => {
        if (match === "%%") {
          return;
        }
        index++;
        if (match === "%c") {
          lastC = index;
        }
      });
      args.splice(lastC, 0, c);
    }
    exports.log = console.debug || console.log || (() => {});
    function save(namespaces) {
      try {
        if (namespaces) {
          exports.storage.setItem("debug", namespaces);
        } else {
          exports.storage.removeItem("debug");
        }
      } catch (error) {}
    }
    function load() {
      let r;
      try {
        r = exports.storage.getItem("debug") || exports.storage.getItem("DEBUG");
      } catch (error) {}
      if (!r && typeof process !== "undefined" && "env" in process) {
        r = process.env.DEBUG;
      }
      return r;
    }
    function localstorage() {
      try {
        return localStorage;
      } catch (error) {}
    }
    module.exports = require_common()(exports);
    var { formatters } = module.exports;
    formatters.j = function(v) {
      try {
        return JSON.stringify(v);
      } catch (error) {
        return "[UnexpectedJSONParseError]: " + error.message;
      }
    };
  });

  // ../../node_modules/.bun/extend@3.0.2/node_modules/extend/index.js
  var require_extend = __commonJS(function(exports, module) {
    var hasOwn = Object.prototype.hasOwnProperty;
    var toStr = Object.prototype.toString;
    var defineProperty = Object.defineProperty;
    var gOPD = Object.getOwnPropertyDescriptor;
    var isArray = function isArray(arr) {
      if (typeof Array.isArray === "function") {
        return Array.isArray(arr);
      }
      return toStr.call(arr) === "[object Array]";
    };
    var isPlainObject = function isPlainObject(obj) {
      if (!obj || toStr.call(obj) !== "[object Object]") {
        return false;
      }
      var hasOwnConstructor = hasOwn.call(obj, "constructor");
      var hasIsPrototypeOf = obj.constructor && obj.constructor.prototype && hasOwn.call(obj.constructor.prototype, "isPrototypeOf");
      if (obj.constructor && !hasOwnConstructor && !hasIsPrototypeOf) {
        return false;
      }
      var key;
      for (key in obj) {}
      return typeof key === "undefined" || hasOwn.call(obj, key);
    };
    var setProperty = function setProperty(target, options) {
      if (defineProperty && options.name === "__proto__") {
        defineProperty(target, options.name, {
          enumerable: true,
          configurable: true,
          value: options.newValue,
          writable: true
        });
      } else {
        target[options.name] = options.newValue;
      }
    };
    var getProperty = function getProperty(obj, name) {
      if (name === "__proto__") {
        if (!hasOwn.call(obj, name)) {
          return;
        } else if (gOPD) {
          return gOPD(obj, name).value;
        }
      }
      return obj[name];
    };
    module.exports = function extend() {
      var options, name, src, copy, copyIsArray, clone;
      var target = arguments[0];
      var i = 1;
      var length = arguments.length;
      var deep = false;
      if (typeof target === "boolean") {
        deep = target;
        target = arguments[1] || {};
        i = 2;
      }
      if (target == null || typeof target !== "object" && typeof target !== "function") {
        target = {};
      }
      for (;i < length; ++i) {
        options = arguments[i];
        if (options != null) {
          for (name in options) {
            src = getProperty(target, name);
            copy = getProperty(options, name);
            if (target !== copy) {
              if (deep && copy && (isPlainObject(copy) || (copyIsArray = isArray(copy)))) {
                if (copyIsArray) {
                  copyIsArray = false;
                  clone = src && isArray(src) ? src : [];
                } else {
                  clone = src && isPlainObject(src) ? src : {};
                }
                setProperty(target, { name, newValue: extend(deep, clone, copy) });
              } else if (typeof copy !== "undefined") {
                setProperty(target, { name, newValue: copy });
              }
            }
          }
        }
      }
      return target;
    };
  });

  // src/ipc-error.ts
  class IpcCallError extends Error {
    code;
    field;
    details;
    auditId;
    constructor(message, code, field = null, details = [], auditId = null) {
      super(message);
      this.code = code;
      this.field = field;
      this.details = details;
      this.auditId = auditId;
      this.name = "IpcCallError";
    }
  }
  function needsConfirmation(e) {
    return e instanceof IpcCallError && e.code === "NEEDS_CONFIRMATION";
  }
  function ipcErrorDetails(err) {
    if (err && typeof err === "object") {
      const details = err.details;
      if (Array.isArray(details))
        return details.filter((d) => typeof d === "string");
    }
    return [];
  }

  // src/iconNames.ts
  var ICON_NAMES = [
    "activity",
    "archive",
    "at-sign",
    "bar-chart-3",
    "bell",
    "book-open",
    "box",
    "bug",
    "calendar",
    "check-square",
    "circle-check",
    "circle-dot",
    "circle-stop",
    "columns-3",
    "copy",
    "database",
    "download",
    "external-link",
    "file-text",
    "flag",
    "folder",
    "folder-tree",
    "git-branch",
    "git-commit",
    "git-compare",
    "git-merge",
    "git-pull-request",
    "history",
    "inbox",
    "layout-dashboard",
    "link",
    "list-tree",
    "maximize-2",
    "message-square",
    "package",
    "pencil",
    "play",
    "plus",
    "refresh-cw",
    "server",
    "settings",
    "shield",
    "sparkles",
    "star",
    "tag",
    "terminal",
    "ticket",
    "trash-2",
    "triangle-alert",
    "user",
    "zap"
  ];
  var NAMES = new Set(ICON_NAMES);
  function isIconName(name) {
    return NAMES.has(name);
  }

  // src/revision.ts
  var WORKING = "working";
  function gitRevision(rev) {
    return `git:${rev}`;
  }
  function parseRevision(raw) {
    if (typeof raw !== "string")
      return null;
    if (raw === WORKING)
      return raw;
    if (/^snap:\d+$/.test(raw))
      return raw;
    if (/^[a-z][a-z0-9_]*:.+$/.test(raw) && !raw.startsWith("snap:"))
      return raw;
    return null;
  }
  function revisionSlot(rev) {
    return rev === WORKING ? null : rev;
  }
  function revisionFromSlot(slot) {
    return slot === null ? WORKING : parseRevision(slot) ?? WORKING;
  }

  // src/filesystem.ts
  var LOCAL_FS = "oxplow:local";
  var MAX_SHOWN_BYTES = 5 * 1024 * 1024;

  // src/uri/uri.ts
  var NAMESPACE = /^[a-z][a-z0-9_]*$/;
  var NAME = /^[a-z][a-z0-9_]*$/;
  var KEY = /^[A-Za-z][A-Za-z0-9_.-]*$/;
  var RAW = /^[A-Za-z0-9\-._~/:?,@!$'()*;]$/;
  function parse(text) {
    if (text === "")
      return { ok: false, reason: "empty" };
    if (!text.startsWith("oxplow:"))
      return { ok: false, reason: "not_oxplow" };
    const rest = text.slice("oxplow:".length);
    if (rest.startsWith("//"))
      return { ok: false, reason: "authority" };
    const hash = rest.indexOf("#");
    const beforeView = hash < 0 ? rest : rest.slice(0, hash);
    const viewText = hash < 0 ? "" : rest.slice(hash + 1);
    const q = beforeView.indexOf("?");
    const path = q < 0 ? beforeView : beforeView.slice(0, q);
    const query = q < 0 ? "" : beforeView.slice(q + 1);
    const slash = path.indexOf("/");
    const namespace = slash < 0 ? "" : path.slice(0, slash);
    const name = slash < 0 ? "" : path.slice(slash + 1);
    if (!NAMESPACE.test(namespace) || !NAME.test(name))
      return { ok: false, reason: "bad_resource" };
    const params = pairs(query);
    if ("reason" in params)
      return { ok: false, reason: params.reason };
    const view = pairs(viewText);
    if ("reason" in view)
      return { ok: false, reason: view.reason };
    return { ok: true, uri: { resource: { namespace, name }, params: params.pairs, view: view.pairs } };
  }
  function pairs(text) {
    const out = {};
    if (text === "")
      return { ok: true, pairs: out };
    for (const pair of text.split("&")) {
      const eq = pair.indexOf("=");
      const key = eq < 0 ? "" : pair.slice(0, eq);
      if (!KEY.test(key))
        return { ok: false, reason: "bad_param" };
      const value = decode(pair.slice(eq + 1));
      if (typeof value !== "string")
        return { ok: false, reason: value.reason };
      if (Object.prototype.hasOwnProperty.call(out, key))
        return { ok: false, reason: "duplicate_param" };
      out[key] = value;
    }
    return { ok: true, pairs: out };
  }
  function decode(value) {
    const bytes = [];
    for (let i = 0;i < value.length; ) {
      const c = value[i];
      if (c === "%") {
        const hex = value.slice(i + 1, i + 3);
        if (!/^[0-9A-Fa-f]{2}$/.test(hex))
          return { ok: false, reason: "bad_escape" };
        bytes.push(parseInt(hex, 16));
        i += 3;
      } else if (RAW.test(c) || c === "+") {
        bytes.push(c.charCodeAt(0));
        i += 1;
      } else {
        return { ok: false, reason: "unescaped" };
      }
    }
    try {
      return new TextDecoder("utf-8", { fatal: true }).decode(new Uint8Array(bytes));
    } catch {
      return { ok: false, reason: "not_utf8" };
    }
  }
  function encode(value) {
    let out = "";
    for (const b of new TextEncoder().encode(value)) {
      const c = String.fromCharCode(b);
      out += b < 128 && RAW.test(c) ? c : `%${b.toString(16).toUpperCase().padStart(2, "0")}`;
    }
    return out;
  }
  function writePairs(lead, pairs) {
    const keys = Object.keys(pairs).sort();
    return keys.length === 0 ? "" : lead + keys.map((k) => `${k}=${encode(pairs[k])}`).join("&");
  }
  function parseUri(text) {
    const p = parse(text);
    return "uri" in p ? p.uri : null;
  }
  function formatUri(u) {
    for (const k of [...Object.keys(u.params), ...Object.keys(u.view)]) {
      if (!KEY.test(k))
        throw new Error(`\`${k}\` isn't a URI param key`);
    }
    return `oxplow:${u.resource.namespace}/${u.resource.name}${writePairs("?", u.params)}${writePairs("#", u.view)}`;
  }
  function isOxplowScheme(text) {
    return text.startsWith("oxplow:");
  }
  function resourceUri(resource, params = {}, view = {}) {
    const slash = resource.indexOf("/");
    const namespace = resource.slice(0, slash);
    const name = resource.slice(slash + 1);
    if (slash < 0 || !NAMESPACE.test(namespace) || !NAME.test(name))
      throw new Error(`\`${resource}\` isn't <namespace>/<resource>`);
    return formatUri({ resource: { namespace, name }, params: present(params), view: present(view) });
  }
  function present(params) {
    const out = {};
    for (const [k, v] of Object.entries(params))
      if (v !== null && v !== undefined)
        out[k] = String(v);
    return out;
  }
  function identityOf(text) {
    const u = parseUri(text);
    return u ? formatUri({ ...u, view: {} }) : null;
  }
  function typeOf(text) {
    const u = parseUri(text);
    return u ? `${u.resource.namespace}/${u.resource.name}` : null;
  }
  function taskOf(text) {
    const u = parseUri(text);
    if (!u || typeOf(text) !== "oxplow/task")
      return null;
    const { provider, id } = u.params;
    return provider && id ? { provider, id } : null;
  }
  function linkTarget(uri) {
    return uri.replace(/\(/g, "%28").replace(/\)/g, "%29");
  }
  var knowledgeUri = (store, id) => resourceUri("oxplow/knowledge", { store, id });
  var commitUri = (sha) => resourceUri("oxplow/commit", { sha });
  var filesystemParam = (filesystem) => filesystem === LOCAL_FS ? null : filesystem;
  var fileUri = (path, rev = null, filesystem = null) => resourceUri("oxplow/file", { path, rev, filesystem: filesystemParam(filesystem) });
  var dirUri = (path, rev = null, filesystem = null) => resourceUri("oxplow/dir", { path, rev, filesystem: filesystemParam(filesystem) });
  function fileLinesUri(path, start, end, rev = null, filesystem = null) {
    return resourceUri("oxplow/file", { path, rev, filesystem: filesystemParam(filesystem) }, { lines: `${start}-${end}` });
  }
  function withUriView(text, view) {
    const u = parseUri(text);
    return u ? formatUri({ ...u, view: present(view) }) : text;
  }
  var CONTRIBUTIONS = new Set([
    "dataset",
    "measure",
    "metric",
    "dimension",
    "collector",
    "event_type",
    "ref_kind",
    "command",
    "job",
    "ai_profile",
    "effect",
    "advisory",
    "agent_context",
    "skill",
    "implementation",
    "provider",
    "page",
    "panel"
  ]);
  var GLOBAL_CONTRIBUTIONS = new Set(["command", "measure", "metric", "dimension", "event_type", "ref_kind"]);

  // src/viz/componentBridge.ts
  var BRIDGE_PROTOCOL = 13;
  var rowsOf = (r, error = null) => ({
    columns: r.columns,
    rows: r.rows,
    truncated: r.truncated,
    ...error ? { error } : {}
  });
  function pageData(run, props, own = null) {
    const data = {};
    for (const [name, l] of Object.entries(run.loaders))
      if (l)
        data[name] = rowsOf(l.result, l.error);
    return { run: own, data, params: run.params, props: props ?? null };
  }
  var isCell = (v) => v === null || typeof v === "string" || typeof v === "number" || typeof v === "boolean";
  function parseFrameMessage(data) {
    if (!data || typeof data !== "object")
      return null;
    const m = data;
    if (m.type === "ready")
      return { type: "ready" };
    if (m.type === "size") {
      return typeof m.height === "number" && Number.isFinite(m.height) && m.height >= 0 ? { type: "size", height: m.height } : null;
    }
    if (typeof m.id !== "string" || !m.id)
      return null;
    switch (m.method) {
      case "invoke":
        if (typeof m.command !== "string")
          return null;
        return { type: "request", request: { id: m.id, method: "invoke", command: m.command, input: m.input ?? {} } };
      case "navigate":
        if (typeof m.uri !== "string")
          return null;
        if (m.newTab !== undefined && typeof m.newTab !== "boolean")
          return null;
        if (m.replace !== undefined && typeof m.replace !== "boolean")
          return null;
        return {
          type: "request",
          request: { id: m.id, method: "navigate", uri: m.uri, newTab: m.newTab === true, replace: m.replace === true }
        };
      case "markdown":
        if (typeof m.text !== "string" || m.inline !== undefined && typeof m.inline !== "boolean")
          return null;
        return { type: "request", request: { id: m.id, method: "markdown", text: m.text, inline: m.inline === true } };
      case "querySql": {
        const params = m.params ?? [];
        if (typeof m.sql !== "string" || !Array.isArray(params) || !params.every(isCell))
          return null;
        return { type: "request", request: { id: m.id, method: "querySql", sql: m.sql, params } };
      }
      case "openForm": {
        if (m.form !== undefined) {
          const f = m.form;
          if (!f || typeof f !== "object" || Array.isArray(f))
            return null;
          const values = f.values ?? {};
          if (typeof f.title !== "string" || !f.schema || typeof f.schema !== "object" || Array.isArray(f.schema))
            return null;
          if (!values || typeof values !== "object" || Array.isArray(values))
            return null;
          return {
            type: "request",
            request: {
              id: m.id,
              method: "openForm",
              form: { title: f.title, schema: f.schema, values }
            }
          };
        }
        if (typeof m.command !== "string")
          return null;
        return { type: "request", request: { id: m.id, method: "openForm", command: m.command, input: m.input ?? {} } };
      }
      case "signIn":
      case "cancelSignIn":
        if (typeof m.instance !== "string" || !m.instance || typeof m.name !== "string" || !m.name)
          return null;
        return { type: "request", request: { id: m.id, method: m.method, instance: m.instance, name: m.name } };
      case "setTitle":
        if (typeof m.title !== "string")
          return null;
        return { type: "request", request: { id: m.id, method: "setTitle", title: m.title } };
      case "setActions": {
        if (!Array.isArray(m.actions))
          return null;
        const actions = [];
        for (const raw of m.actions) {
          const a = raw;
          if (!a || typeof a.id !== "string" || !a.id || typeof a.label !== "string")
            return null;
          actions.push({
            id: a.id,
            label: a.label,
            primary: a.primary === true,
            prominent: a.prominent === true,
            disabled: a.disabled === true,
            ...typeof a.submenu === "string" && a.submenu.trim() ? { submenu: a.submenu.trim() } : {},
            ...typeof a.icon === "string" && isIconName(a.icon) ? { icon: a.icon } : {}
          });
        }
        return { type: "request", request: { id: m.id, method: "setActions", actions } };
      }
      case "setViewParams": {
        const params = m.params;
        if (!params || typeof params !== "object" || Array.isArray(params))
          return null;
        if (!Object.values(params).every((v) => v === null || typeof v === "string"))
          return null;
        return { type: "request", request: { id: m.id, method: "setViewParams", params } };
      }
      case "setHistoryState": {
        const patch = m.patch;
        if (!patch || typeof patch !== "object" || Array.isArray(patch))
          return null;
        return { type: "request", request: { id: m.id, method: "setHistoryState", patch } };
      }
      case "pageMemoryGet":
        if (typeof m.key !== "string" || !m.key)
          return null;
        return { type: "request", request: { id: m.id, method: "pageMemoryGet", key: m.key } };
      case "pageMemorySet":
        if (typeof m.key !== "string" || !m.key)
          return null;
        return { type: "request", request: { id: m.id, method: "pageMemorySet", key: m.key, value: m.value ?? null } };
      case "showView": {
        const spec = m.spec ?? null;
        if (spec !== null && typeof spec !== "string")
          return null;
        return { type: "request", request: { id: m.id, method: "showView", spec } };
      }
      default:
        return null;
    }
  }
  function initMessage(fed, tokens, page = null, view = null) {
    return {
      type: "init",
      protocol: BRIDGE_PROTOCOL,
      run: fed.run,
      data: fed.data,
      params: fed.params,
      props: fed.props ?? null,
      page,
      view,
      tokens
    };
  }
  var PAGE_ONLY = "Only a component shown as a page sets its title, actions or the view it shows.";
  var NO_FORMS = "This host shows no forms and signs in for no component.";
  var NO_VIEW = "This host gives the component no view: only a page, a replacement or a viz's own page does.";
  function createBridgeHost(port, deps, sent, sentView = null) {
    let lastData = Object.fromEntries(Object.entries(sent.data).map(([k, v]) => [k, JSON.stringify(v)]));
    let lastParams = JSON.stringify(sent.params);
    let lastRun = JSON.stringify(sent.run ? [sent.run.params, sent.run.result] : null);
    let lastView = sentView ? JSON.stringify(sentView.viewParams) : null;
    let lastState = sentView ? JSON.stringify(sentView.historyState) : null;
    let ready = false;
    const reply = (id, result) => port.postMessage({ id, ok: true, result });
    const fail = (id, code, message, field = null) => port.postMessage({ id, ok: false, error: field === null ? { code, message } : { code, message, field } });
    const failWith = (id, e) => {
      const code = e?.code;
      const field = e?.field;
      fail(id, typeof code === "string" ? code : "FAILED", e instanceof Error ? e.message : String(e), typeof field === "string" && field ? field : null);
    };
    const handle = async (r) => {
      switch (r.method) {
        case "invoke":
          try {
            reply(r.id, (await deps.invoke(r.command, r.input, false)).result);
          } catch (e) {
            if (!needsConfirmation(e))
              return failWith(r.id, e);
            if (!await deps.confirm(r.command, ipcErrorDetails(e))) {
              return fail(r.id, "CANCELLED", `The person didn't confirm \`${r.command}\`.`);
            }
            try {
              reply(r.id, (await deps.invoke(r.command, r.input, true)).result);
            } catch (again) {
              failWith(r.id, again);
            }
          }
          return;
        case "navigate":
          if (deps.navigate(r.uri, { newTab: r.newTab, replace: r.replace }))
            reply(r.id, null);
          else
            fail(r.id, "INVALID", "A component may open oxplow's pages only.");
          return;
        case "markdown":
          try {
            reply(r.id, await deps.markdown(r.text, r.inline));
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "querySql":
          if (!deps.querySql)
            return fail(r.id, "INVALID", "This host runs no SQL for a component.");
          try {
            reply(r.id, await deps.querySql(r.sql, r.params));
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "openForm": {
          if ("form" in r) {
            if (!deps.askForm)
              return fail(r.id, "INVALID", NO_FORMS);
            try {
              const values = await deps.askForm(r.form);
              if (values === null)
                return fail(r.id, "CANCELLED", "The person closed the form.");
              reply(r.id, values);
            } catch (e) {
              failWith(r.id, e);
            }
            return;
          }
          if (!deps.openForm)
            return fail(r.id, "INVALID", NO_FORMS);
          try {
            const out = await deps.openForm(r.command, r.input);
            if (out === null)
              return fail(r.id, "CANCELLED", `The person closed the form for \`${r.command}\`.`);
            reply(r.id, out.result);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        }
        case "signIn":
          if (!deps.signIn)
            return fail(r.id, "INVALID", NO_FORMS);
          try {
            reply(r.id, await deps.signIn(r.instance, r.name));
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "cancelSignIn":
          if (!deps.cancelSignIn)
            return fail(r.id, "INVALID", NO_FORMS);
          try {
            await deps.cancelSignIn(r.instance, r.name);
            reply(r.id, null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "setTitle":
          if (!deps.setTitle)
            return fail(r.id, "INVALID", PAGE_ONLY);
          deps.setTitle(r.title);
          return reply(r.id, null);
        case "setActions":
          if (!deps.setActions)
            return fail(r.id, "INVALID", PAGE_ONLY);
          deps.setActions(r.actions);
          return reply(r.id, null);
        case "setViewParams":
          if (!deps.setViewParams)
            return fail(r.id, "INVALID", NO_VIEW);
          try {
            deps.setViewParams(r.params);
            reply(r.id, null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "setHistoryState":
          if (!deps.setHistoryState)
            return fail(r.id, "INVALID", NO_VIEW);
          try {
            deps.setHistoryState(r.patch);
            reply(r.id, null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "showView":
          if (!deps.showView)
            return fail(r.id, "INVALID", PAGE_ONLY);
          try {
            reply(r.id, await deps.showView(r.spec) ?? null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "pageMemoryGet":
          if (!deps.pageMemoryGet)
            return fail(r.id, "INVALID", "This host keeps no page memory.");
          try {
            reply(r.id, await deps.pageMemoryGet(r.key) ?? null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
        case "pageMemorySet":
          if (!deps.pageMemorySet)
            return fail(r.id, "INVALID", "This host keeps no page memory.");
          try {
            deps.pageMemorySet(r.key, r.value);
            reply(r.id, null);
          } catch (e) {
            failWith(r.id, e);
          }
          return;
      }
    };
    port.onmessage = (e) => {
      const m = parseFrameMessage(e.data);
      if (m?.type === "ready") {
        if (ready)
          return;
        ready = true;
        return deps.onReady();
      }
      if (m?.type === "size")
        return void deps.size?.(m.height);
      if (m?.type === "request")
        return void handle(m.request);
      const id = e.data?.id;
      if (typeof id === "string" && id)
        fail(id, "BAD_REQUEST", "Not a request this host knows.");
    };
    return {
      update: (fed) => {
        const data = Object.fromEntries(Object.entries(fed.data).map(([k, v]) => [k, JSON.stringify(v)]));
        const changed = [...new Set([...Object.keys(data), ...Object.keys(lastData)])].filter((k) => data[k] !== lastData[k]);
        const params = JSON.stringify(fed.params);
        const run = JSON.stringify(fed.run ? [fed.run.params, fed.run.result] : null);
        if (changed.length === 0 && params === lastParams && run === lastRun)
          return;
        lastData = data;
        lastParams = params;
        lastRun = run;
        port.postMessage({ type: "update", run: fed.run, data: fed.data, params: fed.params, changed });
      },
      viewParams: (params) => {
        const key = JSON.stringify(params);
        if (key === lastView)
          return;
        lastView = key;
        port.postMessage({ type: "viewParams", params });
      },
      historyState: (state) => {
        const key = JSON.stringify(state);
        if (key === lastState)
          return;
        lastState = key;
        port.postMessage({ type: "historyState", state });
      },
      action: (id) => port.postMessage({ type: "action", id }),
      close: () => {
        port.close();
        deps.onClose?.();
      }
    };
  }

  // src/viz/nodeRun.ts
  var EMPTY_RESULT = {
    columns: [],
    rows: [],
    truncated: false,
    reads: { datasets: [], tables: [] },
    freshness: []
  };
  function nodeRunOf(run, node) {
    if (node.kind !== "viz")
      return null;
    return {
      config: node.config,
      params: run.params,
      result: node.from !== null && run.loaders[node.from]?.result || EMPTY_RESULT
    };
  }
  function nodeById(page, id) {
    return find(page.view, id) ?? (page.rail ? find(page.rail, id) : null);
  }
  function find(at, id) {
    if (at.id === id)
      return at;
    if (at.kind === "stack" || at.kind === "grid" || at.kind === "switch") {
      for (const c of at.children) {
        const hit = find(c, id);
        if (hit)
          return hit;
      }
    }
    return null;
  }

  // ../../node_modules/.bun/ccount@2.0.1/node_modules/ccount/index.js
  function ccount(value, character) {
    const source = String(value);
    if (typeof character !== "string") {
      throw new TypeError("Expected character");
    }
    let count = 0;
    let index = source.indexOf(character);
    while (index !== -1) {
      count++;
      index = source.indexOf(character, index + character.length);
    }
    return count;
  }

  // ../../node_modules/.bun/devlop@1.1.0/node_modules/devlop/lib/development.js
  var codesWarned = new Set;

  class AssertionError extends Error {
    name = "Assertion";
    code = "ERR_ASSERTION";
    constructor(message, actual, expected, operator, generated) {
      super(message);
      if (Error.captureStackTrace) {
        Error.captureStackTrace(this, this.constructor);
      }
      this.actual = actual;
      this.expected = expected;
      this.generated = generated;
      this.operator = operator;
    }
  }
  function ok(value, message) {
    assert(Boolean(value), false, true, "ok", "Expected value to be truthy", message);
  }
  function assert(bool, actual, expected, operator, defaultMessage, userMessage) {
    if (!bool) {
      throw userMessage instanceof Error ? userMessage : new AssertionError(userMessage || defaultMessage, actual, expected, operator, !userMessage);
    }
  }

  // ../../node_modules/.bun/micromark-util-symbol@2.0.1/node_modules/micromark-util-symbol/lib/codes.js
  var codes = {
    carriageReturn: -5,
    lineFeed: -4,
    carriageReturnLineFeed: -3,
    horizontalTab: -2,
    virtualSpace: -1,
    eof: null,
    nul: 0,
    soh: 1,
    stx: 2,
    etx: 3,
    eot: 4,
    enq: 5,
    ack: 6,
    bel: 7,
    bs: 8,
    ht: 9,
    lf: 10,
    vt: 11,
    ff: 12,
    cr: 13,
    so: 14,
    si: 15,
    dle: 16,
    dc1: 17,
    dc2: 18,
    dc3: 19,
    dc4: 20,
    nak: 21,
    syn: 22,
    etb: 23,
    can: 24,
    em: 25,
    sub: 26,
    esc: 27,
    fs: 28,
    gs: 29,
    rs: 30,
    us: 31,
    space: 32,
    exclamationMark: 33,
    quotationMark: 34,
    numberSign: 35,
    dollarSign: 36,
    percentSign: 37,
    ampersand: 38,
    apostrophe: 39,
    leftParenthesis: 40,
    rightParenthesis: 41,
    asterisk: 42,
    plusSign: 43,
    comma: 44,
    dash: 45,
    dot: 46,
    slash: 47,
    digit0: 48,
    digit1: 49,
    digit2: 50,
    digit3: 51,
    digit4: 52,
    digit5: 53,
    digit6: 54,
    digit7: 55,
    digit8: 56,
    digit9: 57,
    colon: 58,
    semicolon: 59,
    lessThan: 60,
    equalsTo: 61,
    greaterThan: 62,
    questionMark: 63,
    atSign: 64,
    uppercaseA: 65,
    uppercaseB: 66,
    uppercaseC: 67,
    uppercaseD: 68,
    uppercaseE: 69,
    uppercaseF: 70,
    uppercaseG: 71,
    uppercaseH: 72,
    uppercaseI: 73,
    uppercaseJ: 74,
    uppercaseK: 75,
    uppercaseL: 76,
    uppercaseM: 77,
    uppercaseN: 78,
    uppercaseO: 79,
    uppercaseP: 80,
    uppercaseQ: 81,
    uppercaseR: 82,
    uppercaseS: 83,
    uppercaseT: 84,
    uppercaseU: 85,
    uppercaseV: 86,
    uppercaseW: 87,
    uppercaseX: 88,
    uppercaseY: 89,
    uppercaseZ: 90,
    leftSquareBracket: 91,
    backslash: 92,
    rightSquareBracket: 93,
    caret: 94,
    underscore: 95,
    graveAccent: 96,
    lowercaseA: 97,
    lowercaseB: 98,
    lowercaseC: 99,
    lowercaseD: 100,
    lowercaseE: 101,
    lowercaseF: 102,
    lowercaseG: 103,
    lowercaseH: 104,
    lowercaseI: 105,
    lowercaseJ: 106,
    lowercaseK: 107,
    lowercaseL: 108,
    lowercaseM: 109,
    lowercaseN: 110,
    lowercaseO: 111,
    lowercaseP: 112,
    lowercaseQ: 113,
    lowercaseR: 114,
    lowercaseS: 115,
    lowercaseT: 116,
    lowercaseU: 117,
    lowercaseV: 118,
    lowercaseW: 119,
    lowercaseX: 120,
    lowercaseY: 121,
    lowercaseZ: 122,
    leftCurlyBrace: 123,
    verticalBar: 124,
    rightCurlyBrace: 125,
    tilde: 126,
    del: 127,
    byteOrderMarker: 65279,
    replacementCharacter: 65533
  };
  // ../../node_modules/.bun/micromark-util-symbol@2.0.1/node_modules/micromark-util-symbol/lib/constants.js
  var constants = {
    attentionSideAfter: 2,
    attentionSideBefore: 1,
    atxHeadingOpeningFenceSizeMax: 6,
    autolinkDomainSizeMax: 63,
    autolinkSchemeSizeMax: 32,
    cdataOpeningString: "CDATA[",
    characterGroupPunctuation: 2,
    characterGroupWhitespace: 1,
    characterReferenceDecimalSizeMax: 7,
    characterReferenceHexadecimalSizeMax: 6,
    characterReferenceNamedSizeMax: 31,
    codeFencedSequenceSizeMin: 3,
    contentTypeContent: "content",
    contentTypeDocument: "document",
    contentTypeFlow: "flow",
    contentTypeString: "string",
    contentTypeText: "text",
    hardBreakPrefixSizeMin: 2,
    htmlBasic: 6,
    htmlCdata: 5,
    htmlComment: 2,
    htmlComplete: 7,
    htmlDeclaration: 4,
    htmlInstruction: 3,
    htmlRawSizeMax: 8,
    htmlRaw: 1,
    linkResourceDestinationBalanceMax: 32,
    linkReferenceSizeMax: 999,
    listItemValueSizeMax: 10,
    numericBaseDecimal: 10,
    numericBaseHexadecimal: 16,
    tabSize: 4,
    thematicBreakMarkerCountMin: 3,
    v8MaxSafeChunkSize: 1e4
  };
  // ../../node_modules/.bun/micromark-util-symbol@2.0.1/node_modules/micromark-util-symbol/lib/types.js
  var types = {
    data: "data",
    whitespace: "whitespace",
    lineEnding: "lineEnding",
    lineEndingBlank: "lineEndingBlank",
    linePrefix: "linePrefix",
    lineSuffix: "lineSuffix",
    atxHeading: "atxHeading",
    atxHeadingSequence: "atxHeadingSequence",
    atxHeadingText: "atxHeadingText",
    autolink: "autolink",
    autolinkEmail: "autolinkEmail",
    autolinkMarker: "autolinkMarker",
    autolinkProtocol: "autolinkProtocol",
    characterEscape: "characterEscape",
    characterEscapeValue: "characterEscapeValue",
    characterReference: "characterReference",
    characterReferenceMarker: "characterReferenceMarker",
    characterReferenceMarkerNumeric: "characterReferenceMarkerNumeric",
    characterReferenceMarkerHexadecimal: "characterReferenceMarkerHexadecimal",
    characterReferenceValue: "characterReferenceValue",
    codeFenced: "codeFenced",
    codeFencedFence: "codeFencedFence",
    codeFencedFenceSequence: "codeFencedFenceSequence",
    codeFencedFenceInfo: "codeFencedFenceInfo",
    codeFencedFenceMeta: "codeFencedFenceMeta",
    codeFlowValue: "codeFlowValue",
    codeIndented: "codeIndented",
    codeText: "codeText",
    codeTextData: "codeTextData",
    codeTextPadding: "codeTextPadding",
    codeTextSequence: "codeTextSequence",
    content: "content",
    definition: "definition",
    definitionDestination: "definitionDestination",
    definitionDestinationLiteral: "definitionDestinationLiteral",
    definitionDestinationLiteralMarker: "definitionDestinationLiteralMarker",
    definitionDestinationRaw: "definitionDestinationRaw",
    definitionDestinationString: "definitionDestinationString",
    definitionLabel: "definitionLabel",
    definitionLabelMarker: "definitionLabelMarker",
    definitionLabelString: "definitionLabelString",
    definitionMarker: "definitionMarker",
    definitionTitle: "definitionTitle",
    definitionTitleMarker: "definitionTitleMarker",
    definitionTitleString: "definitionTitleString",
    emphasis: "emphasis",
    emphasisSequence: "emphasisSequence",
    emphasisText: "emphasisText",
    escapeMarker: "escapeMarker",
    hardBreakEscape: "hardBreakEscape",
    hardBreakTrailing: "hardBreakTrailing",
    htmlFlow: "htmlFlow",
    htmlFlowData: "htmlFlowData",
    htmlText: "htmlText",
    htmlTextData: "htmlTextData",
    image: "image",
    label: "label",
    labelText: "labelText",
    labelLink: "labelLink",
    labelImage: "labelImage",
    labelMarker: "labelMarker",
    labelImageMarker: "labelImageMarker",
    labelEnd: "labelEnd",
    link: "link",
    paragraph: "paragraph",
    reference: "reference",
    referenceMarker: "referenceMarker",
    referenceString: "referenceString",
    resource: "resource",
    resourceDestination: "resourceDestination",
    resourceDestinationLiteral: "resourceDestinationLiteral",
    resourceDestinationLiteralMarker: "resourceDestinationLiteralMarker",
    resourceDestinationRaw: "resourceDestinationRaw",
    resourceDestinationString: "resourceDestinationString",
    resourceMarker: "resourceMarker",
    resourceTitle: "resourceTitle",
    resourceTitleMarker: "resourceTitleMarker",
    resourceTitleString: "resourceTitleString",
    setextHeading: "setextHeading",
    setextHeadingText: "setextHeadingText",
    setextHeadingLine: "setextHeadingLine",
    setextHeadingLineSequence: "setextHeadingLineSequence",
    strong: "strong",
    strongSequence: "strongSequence",
    strongText: "strongText",
    thematicBreak: "thematicBreak",
    thematicBreakSequence: "thematicBreakSequence",
    blockQuote: "blockQuote",
    blockQuotePrefix: "blockQuotePrefix",
    blockQuoteMarker: "blockQuoteMarker",
    blockQuotePrefixWhitespace: "blockQuotePrefixWhitespace",
    listOrdered: "listOrdered",
    listUnordered: "listUnordered",
    listItemIndent: "listItemIndent",
    listItemMarker: "listItemMarker",
    listItemPrefix: "listItemPrefix",
    listItemPrefixWhitespace: "listItemPrefixWhitespace",
    listItemValue: "listItemValue",
    chunkDocument: "chunkDocument",
    chunkContent: "chunkContent",
    chunkFlow: "chunkFlow",
    chunkText: "chunkText",
    chunkString: "chunkString"
  };
  // ../../node_modules/.bun/micromark-util-symbol@2.0.1/node_modules/micromark-util-symbol/lib/values.js
  var values = {
    ht: "\t",
    lf: `
`,
    cr: "\r",
    space: " ",
    exclamationMark: "!",
    quotationMark: '"',
    numberSign: "#",
    dollarSign: "$",
    percentSign: "%",
    ampersand: "&",
    apostrophe: "'",
    leftParenthesis: "(",
    rightParenthesis: ")",
    asterisk: "*",
    plusSign: "+",
    comma: ",",
    dash: "-",
    dot: ".",
    slash: "/",
    digit0: "0",
    digit1: "1",
    digit2: "2",
    digit3: "3",
    digit4: "4",
    digit5: "5",
    digit6: "6",
    digit7: "7",
    digit8: "8",
    digit9: "9",
    colon: ":",
    semicolon: ";",
    lessThan: "<",
    equalsTo: "=",
    greaterThan: ">",
    questionMark: "?",
    atSign: "@",
    uppercaseA: "A",
    uppercaseB: "B",
    uppercaseC: "C",
    uppercaseD: "D",
    uppercaseE: "E",
    uppercaseF: "F",
    uppercaseG: "G",
    uppercaseH: "H",
    uppercaseI: "I",
    uppercaseJ: "J",
    uppercaseK: "K",
    uppercaseL: "L",
    uppercaseM: "M",
    uppercaseN: "N",
    uppercaseO: "O",
    uppercaseP: "P",
    uppercaseQ: "Q",
    uppercaseR: "R",
    uppercaseS: "S",
    uppercaseT: "T",
    uppercaseU: "U",
    uppercaseV: "V",
    uppercaseW: "W",
    uppercaseX: "X",
    uppercaseY: "Y",
    uppercaseZ: "Z",
    leftSquareBracket: "[",
    backslash: "\\",
    rightSquareBracket: "]",
    caret: "^",
    underscore: "_",
    graveAccent: "`",
    lowercaseA: "a",
    lowercaseB: "b",
    lowercaseC: "c",
    lowercaseD: "d",
    lowercaseE: "e",
    lowercaseF: "f",
    lowercaseG: "g",
    lowercaseH: "h",
    lowercaseI: "i",
    lowercaseJ: "j",
    lowercaseK: "k",
    lowercaseL: "l",
    lowercaseM: "m",
    lowercaseN: "n",
    lowercaseO: "o",
    lowercaseP: "p",
    lowercaseQ: "q",
    lowercaseR: "r",
    lowercaseS: "s",
    lowercaseT: "t",
    lowercaseU: "u",
    lowercaseV: "v",
    lowercaseW: "w",
    lowercaseX: "x",
    lowercaseY: "y",
    lowercaseZ: "z",
    leftCurlyBrace: "{",
    verticalBar: "|",
    rightCurlyBrace: "}",
    tilde: "~",
    replacementCharacter: "�"
  };
  // ../../node_modules/.bun/micromark-util-character@2.1.1/node_modules/micromark-util-character/dev/index.js
  var asciiAlpha = regexCheck(/[A-Za-z]/);
  var asciiAlphanumeric = regexCheck(/[\dA-Za-z]/);
  var asciiAtext = regexCheck(/[#-'*+\--9=?A-Z^-~]/);
  function asciiControl(code) {
    return code !== null && (code < codes.space || code === codes.del);
  }
  var asciiDigit = regexCheck(/\d/);
  var asciiHexDigit = regexCheck(/[\dA-Fa-f]/);
  var asciiPunctuation = regexCheck(/[!-/:-@[-`{-~]/);
  function markdownLineEnding(code) {
    return code !== null && code < codes.horizontalTab;
  }
  function markdownLineEndingOrSpace(code) {
    return code !== null && (code < codes.nul || code === codes.space);
  }
  function markdownSpace(code) {
    return code === codes.horizontalTab || code === codes.virtualSpace || code === codes.space;
  }
  var unicodePunctuation = regexCheck(/\p{P}|\p{S}/u);
  var unicodeWhitespace = regexCheck(/\s/);
  function regexCheck(regex) {
    return check;
    function check(code) {
      return code !== null && code > -1 && regex.test(String.fromCharCode(code));
    }
  }

  // ../../node_modules/.bun/escape-string-regexp@5.0.0/node_modules/escape-string-regexp/index.js
  function escapeStringRegexp(string) {
    if (typeof string !== "string") {
      throw new TypeError("Expected a string");
    }
    return string.replace(/[|\\{}()[\]^$+*?.]/g, "\\$&").replace(/-/g, "\\x2d");
  }

  // ../../node_modules/.bun/unist-util-is@6.0.1/node_modules/unist-util-is/lib/index.js
  var convert = function(test) {
    if (test === null || test === undefined) {
      return ok2;
    }
    if (typeof test === "function") {
      return castFactory(test);
    }
    if (typeof test === "object") {
      return Array.isArray(test) ? anyFactory(test) : propertiesFactory(test);
    }
    if (typeof test === "string") {
      return typeFactory(test);
    }
    throw new Error("Expected function, string, or object as test");
  };
  function anyFactory(tests) {
    const checks = [];
    let index = -1;
    while (++index < tests.length) {
      checks[index] = convert(tests[index]);
    }
    return castFactory(any);
    function any(...parameters) {
      let index = -1;
      while (++index < checks.length) {
        if (checks[index].apply(this, parameters))
          return true;
      }
      return false;
    }
  }
  function propertiesFactory(check) {
    const checkAsRecord = check;
    return castFactory(all);
    function all(node) {
      const nodeAsRecord = node;
      let key;
      for (key in check) {
        if (nodeAsRecord[key] !== checkAsRecord[key])
          return false;
      }
      return true;
    }
  }
  function typeFactory(check) {
    return castFactory(type);
    function type(node) {
      return node && node.type === check;
    }
  }
  function castFactory(testFunction) {
    return check;
    function check(value, index, parent) {
      return Boolean(looksLikeANode(value) && testFunction.call(this, value, typeof index === "number" ? index : undefined, parent || undefined));
    }
  }
  function ok2() {
    return true;
  }
  function looksLikeANode(value) {
    return value !== null && typeof value === "object" && "type" in value;
  }
  // ../../node_modules/.bun/unist-util-visit-parents@6.0.2/node_modules/unist-util-visit-parents/lib/color.js
  function color(d) {
    return d;
  }

  // ../../node_modules/.bun/unist-util-visit-parents@6.0.2/node_modules/unist-util-visit-parents/lib/index.js
  var empty = [];
  var CONTINUE = true;
  var EXIT = false;
  var SKIP = "skip";
  function visitParents(tree, test, visitor, reverse) {
    let check;
    if (typeof test === "function" && typeof visitor !== "function") {
      reverse = visitor;
      visitor = test;
    } else {
      check = test;
    }
    const is = convert(check);
    const step = reverse ? -1 : 1;
    factory(tree, undefined, [])();
    function factory(node, index, parents) {
      const value = node && typeof node === "object" ? node : {};
      if (typeof value.type === "string") {
        const name = typeof value.tagName === "string" ? value.tagName : typeof value.name === "string" ? value.name : undefined;
        Object.defineProperty(visit, "name", {
          value: "node (" + color(node.type + (name ? "<" + name + ">" : "")) + ")"
        });
      }
      return visit;
      function visit() {
        let result = empty;
        let subresult;
        let offset;
        let grandparents;
        if (!test || is(node, index, parents[parents.length - 1] || undefined)) {
          result = toResult(visitor(node, parents));
          if (result[0] === EXIT) {
            return result;
          }
        }
        if ("children" in node && node.children) {
          const nodeAsParent = node;
          if (nodeAsParent.children && result[0] !== SKIP) {
            offset = (reverse ? nodeAsParent.children.length : -1) + step;
            grandparents = parents.concat(nodeAsParent);
            while (offset > -1 && offset < nodeAsParent.children.length) {
              const child = nodeAsParent.children[offset];
              subresult = factory(child, offset, grandparents)();
              if (subresult[0] === EXIT) {
                return subresult;
              }
              offset = typeof subresult[1] === "number" ? subresult[1] : offset + step;
            }
          }
        }
        return result;
      }
    }
  }
  function toResult(value) {
    if (Array.isArray(value)) {
      return value;
    }
    if (typeof value === "number") {
      return [CONTINUE, value];
    }
    return value === null || value === undefined ? empty : [value];
  }
  // ../../node_modules/.bun/mdast-util-find-and-replace@3.0.2/node_modules/mdast-util-find-and-replace/lib/index.js
  function findAndReplace(tree, list, options) {
    const settings = options || {};
    const ignored = convert(settings.ignore || []);
    const pairs = toPairs(list);
    let pairIndex = -1;
    while (++pairIndex < pairs.length) {
      visitParents(tree, "text", visitor);
    }
    function visitor(node, parents) {
      let index = -1;
      let grandparent;
      while (++index < parents.length) {
        const parent = parents[index];
        const siblings = grandparent ? grandparent.children : undefined;
        if (ignored(parent, siblings ? siblings.indexOf(parent) : undefined, grandparent)) {
          return;
        }
        grandparent = parent;
      }
      if (grandparent) {
        return handler(node, parents);
      }
    }
    function handler(node, parents) {
      const parent = parents[parents.length - 1];
      const find = pairs[pairIndex][0];
      const replace = pairs[pairIndex][1];
      let start = 0;
      const siblings = parent.children;
      const index = siblings.indexOf(node);
      let change = false;
      let nodes = [];
      find.lastIndex = 0;
      let match = find.exec(node.value);
      while (match) {
        const position = match.index;
        const matchObject = {
          index: match.index,
          input: match.input,
          stack: [...parents, node]
        };
        let value = replace(...match, matchObject);
        if (typeof value === "string") {
          value = value.length > 0 ? { type: "text", value } : undefined;
        }
        if (value === false) {
          find.lastIndex = position + 1;
        } else {
          if (start !== position) {
            nodes.push({
              type: "text",
              value: node.value.slice(start, position)
            });
          }
          if (Array.isArray(value)) {
            nodes.push(...value);
          } else if (value) {
            nodes.push(value);
          }
          start = position + match[0].length;
          change = true;
        }
        if (!find.global) {
          break;
        }
        match = find.exec(node.value);
      }
      if (change) {
        if (start < node.value.length) {
          nodes.push({ type: "text", value: node.value.slice(start) });
        }
        parent.children.splice(index, 1, ...nodes);
      } else {
        nodes = [node];
      }
      return index + nodes.length;
    }
  }
  function toPairs(tupleOrList) {
    const result = [];
    if (!Array.isArray(tupleOrList)) {
      throw new TypeError("Expected find and replace tuple or list of tuples");
    }
    const list = !tupleOrList[0] || Array.isArray(tupleOrList[0]) ? tupleOrList : [tupleOrList];
    let index = -1;
    while (++index < list.length) {
      const tuple = list[index];
      result.push([toExpression(tuple[0]), toFunction(tuple[1])]);
    }
    return result;
  }
  function toExpression(find) {
    return typeof find === "string" ? new RegExp(escapeStringRegexp(find), "g") : find;
  }
  function toFunction(replace) {
    return typeof replace === "function" ? replace : function() {
      return replace;
    };
  }
  // ../../node_modules/.bun/mdast-util-gfm-autolink-literal@2.0.1/node_modules/mdast-util-gfm-autolink-literal/lib/index.js
  var inConstruct = "phrasing";
  var notInConstruct = ["autolink", "link", "image", "label"];
  function gfmAutolinkLiteralFromMarkdown() {
    return {
      transforms: [transformGfmAutolinkLiterals],
      enter: {
        literalAutolink: enterLiteralAutolink,
        literalAutolinkEmail: enterLiteralAutolinkValue,
        literalAutolinkHttp: enterLiteralAutolinkValue,
        literalAutolinkWww: enterLiteralAutolinkValue
      },
      exit: {
        literalAutolink: exitLiteralAutolink,
        literalAutolinkEmail: exitLiteralAutolinkEmail,
        literalAutolinkHttp: exitLiteralAutolinkHttp,
        literalAutolinkWww: exitLiteralAutolinkWww
      }
    };
  }
  function gfmAutolinkLiteralToMarkdown() {
    return {
      unsafe: [
        {
          character: "@",
          before: "[+\\-.\\w]",
          after: "[\\-.\\w]",
          inConstruct,
          notInConstruct
        },
        {
          character: ".",
          before: "[Ww]",
          after: "[\\-.\\w]",
          inConstruct,
          notInConstruct
        },
        {
          character: ":",
          before: "[ps]",
          after: "\\/",
          inConstruct,
          notInConstruct
        }
      ]
    };
  }
  function enterLiteralAutolink(token) {
    this.enter({ type: "link", title: null, url: "", children: [] }, token);
  }
  function enterLiteralAutolinkValue(token) {
    this.config.enter.autolinkProtocol.call(this, token);
  }
  function exitLiteralAutolinkHttp(token) {
    this.config.exit.autolinkProtocol.call(this, token);
  }
  function exitLiteralAutolinkWww(token) {
    this.config.exit.data.call(this, token);
    const node = this.stack[this.stack.length - 1];
    ok(node.type === "link");
    node.url = "http://" + this.sliceSerialize(token);
  }
  function exitLiteralAutolinkEmail(token) {
    this.config.exit.autolinkEmail.call(this, token);
  }
  function exitLiteralAutolink(token) {
    this.exit(token);
  }
  function transformGfmAutolinkLiterals(tree) {
    findAndReplace(tree, [
      [/(https?:\/\/|www(?=\.))([-.\w]+)([^ \t\r\n]*)/gi, findUrl],
      [/(?<=^|\s|\p{P}|\p{S})([-.\w+]+)@([-\w]+(?:\.[-\w]+)+)/gu, findEmail]
    ], { ignore: ["link", "linkReference"] });
  }
  function findUrl(_, protocol, domain, path, match) {
    let prefix = "";
    if (!previous(match)) {
      return false;
    }
    if (/^w/i.test(protocol)) {
      domain = protocol + domain;
      protocol = "";
      prefix = "http://";
    }
    if (!isCorrectDomain(domain)) {
      return false;
    }
    const parts = splitUrl(domain + path);
    if (!parts[0])
      return false;
    const result = {
      type: "link",
      title: null,
      url: prefix + protocol + parts[0],
      children: [{ type: "text", value: protocol + parts[0] }]
    };
    if (parts[1]) {
      return [result, { type: "text", value: parts[1] }];
    }
    return result;
  }
  function findEmail(_, atext, label, match) {
    if (!previous(match, true) || /[-\d_]$/.test(label)) {
      return false;
    }
    return {
      type: "link",
      title: null,
      url: "mailto:" + atext + "@" + label,
      children: [{ type: "text", value: atext + "@" + label }]
    };
  }
  function isCorrectDomain(domain) {
    const parts = domain.split(".");
    if (parts.length < 2 || parts[parts.length - 1] && (/_/.test(parts[parts.length - 1]) || !/[a-zA-Z\d]/.test(parts[parts.length - 1])) || parts[parts.length - 2] && (/_/.test(parts[parts.length - 2]) || !/[a-zA-Z\d]/.test(parts[parts.length - 2]))) {
      return false;
    }
    return true;
  }
  function splitUrl(url) {
    const trailExec = /[!"&'),.:;<>?\]}]+$/.exec(url);
    if (!trailExec) {
      return [url, undefined];
    }
    url = url.slice(0, trailExec.index);
    let trail = trailExec[0];
    let closingParenIndex = trail.indexOf(")");
    const openingParens = ccount(url, "(");
    let closingParens = ccount(url, ")");
    while (closingParenIndex !== -1 && openingParens > closingParens) {
      url += trail.slice(0, closingParenIndex + 1);
      trail = trail.slice(closingParenIndex + 1);
      closingParenIndex = trail.indexOf(")");
      closingParens++;
    }
    return [url, trail];
  }
  function previous(match, email) {
    const code = match.input.charCodeAt(match.index - 1);
    return (match.index === 0 || unicodeWhitespace(code) || unicodePunctuation(code)) && (!email || code !== 47);
  }
  // ../../node_modules/.bun/micromark-util-normalize-identifier@2.0.1/node_modules/micromark-util-normalize-identifier/dev/index.js
  function normalizeIdentifier(value) {
    return value.replace(/[\t\n\r ]+/g, values.space).replace(/^ | $/g, "").toLowerCase().toUpperCase();
  }

  // ../../node_modules/.bun/mdast-util-gfm-footnote@2.1.0/node_modules/mdast-util-gfm-footnote/lib/index.js
  footnoteReference.peek = footnoteReferencePeek;
  function enterFootnoteCallString() {
    this.buffer();
  }
  function enterFootnoteCall(token) {
    this.enter({ type: "footnoteReference", identifier: "", label: "" }, token);
  }
  function enterFootnoteDefinitionLabelString() {
    this.buffer();
  }
  function enterFootnoteDefinition(token) {
    this.enter({ type: "footnoteDefinition", identifier: "", label: "", children: [] }, token);
  }
  function exitFootnoteCallString(token) {
    const label = this.resume();
    const node = this.stack[this.stack.length - 1];
    ok(node.type === "footnoteReference");
    node.identifier = normalizeIdentifier(this.sliceSerialize(token)).toLowerCase();
    node.label = label;
  }
  function exitFootnoteCall(token) {
    this.exit(token);
  }
  function exitFootnoteDefinitionLabelString(token) {
    const label = this.resume();
    const node = this.stack[this.stack.length - 1];
    ok(node.type === "footnoteDefinition");
    node.identifier = normalizeIdentifier(this.sliceSerialize(token)).toLowerCase();
    node.label = label;
  }
  function exitFootnoteDefinition(token) {
    this.exit(token);
  }
  function footnoteReferencePeek() {
    return "[";
  }
  function footnoteReference(node, _, state, info) {
    const tracker = state.createTracker(info);
    let value = tracker.move("[^");
    const exit = state.enter("footnoteReference");
    const subexit = state.enter("reference");
    value += tracker.move(state.safe(state.associationId(node), { after: "]", before: value }));
    subexit();
    exit();
    value += tracker.move("]");
    return value;
  }
  function gfmFootnoteFromMarkdown() {
    return {
      enter: {
        gfmFootnoteCallString: enterFootnoteCallString,
        gfmFootnoteCall: enterFootnoteCall,
        gfmFootnoteDefinitionLabelString: enterFootnoteDefinitionLabelString,
        gfmFootnoteDefinition: enterFootnoteDefinition
      },
      exit: {
        gfmFootnoteCallString: exitFootnoteCallString,
        gfmFootnoteCall: exitFootnoteCall,
        gfmFootnoteDefinitionLabelString: exitFootnoteDefinitionLabelString,
        gfmFootnoteDefinition: exitFootnoteDefinition
      }
    };
  }
  function gfmFootnoteToMarkdown(options) {
    let firstLineBlank = false;
    if (options && options.firstLineBlank) {
      firstLineBlank = true;
    }
    return {
      handlers: { footnoteDefinition, footnoteReference },
      unsafe: [{ character: "[", inConstruct: ["label", "phrasing", "reference"] }]
    };
    function footnoteDefinition(node, _, state, info) {
      const tracker = state.createTracker(info);
      let value = tracker.move("[^");
      const exit = state.enter("footnoteDefinition");
      const subexit = state.enter("label");
      value += tracker.move(state.safe(state.associationId(node), { before: value, after: "]" }));
      subexit();
      value += tracker.move("]:");
      if (node.children && node.children.length > 0) {
        tracker.shift(4);
        value += tracker.move((firstLineBlank ? `
` : " ") + state.indentLines(state.containerFlow(node, tracker.current()), firstLineBlank ? mapAll : mapExceptFirst));
      }
      exit();
      return value;
    }
  }
  function mapExceptFirst(line, index, blank) {
    return index === 0 ? line : mapAll(line, index, blank);
  }
  function mapAll(line, index, blank) {
    return (blank ? "" : "    ") + line;
  }
  // ../../node_modules/.bun/mdast-util-gfm-strikethrough@2.0.0/node_modules/mdast-util-gfm-strikethrough/lib/index.js
  var constructsWithoutStrikethrough = [
    "autolink",
    "destinationLiteral",
    "destinationRaw",
    "reference",
    "titleQuote",
    "titleApostrophe"
  ];
  handleDelete.peek = peekDelete;
  function gfmStrikethroughFromMarkdown() {
    return {
      canContainEols: ["delete"],
      enter: { strikethrough: enterStrikethrough },
      exit: { strikethrough: exitStrikethrough }
    };
  }
  function gfmStrikethroughToMarkdown() {
    return {
      unsafe: [
        {
          character: "~",
          inConstruct: "phrasing",
          notInConstruct: constructsWithoutStrikethrough
        }
      ],
      handlers: { delete: handleDelete }
    };
  }
  function enterStrikethrough(token) {
    this.enter({ type: "delete", children: [] }, token);
  }
  function exitStrikethrough(token) {
    this.exit(token);
  }
  function handleDelete(node, _, state, info) {
    const tracker = state.createTracker(info);
    const exit = state.enter("strikethrough");
    let value = tracker.move("~~");
    value += state.containerPhrasing(node, {
      ...tracker.current(),
      before: value,
      after: "~"
    });
    value += tracker.move("~~");
    exit();
    return value;
  }
  function peekDelete() {
    return "~";
  }
  // ../../node_modules/.bun/markdown-table@3.0.4/node_modules/markdown-table/index.js
  function defaultStringLength(value) {
    return value.length;
  }
  function markdownTable(table, options) {
    const settings = options || {};
    const align = (settings.align || []).concat();
    const stringLength = settings.stringLength || defaultStringLength;
    const alignments = [];
    const cellMatrix = [];
    const sizeMatrix = [];
    const longestCellByColumn = [];
    let mostCellsPerRow = 0;
    let rowIndex = -1;
    while (++rowIndex < table.length) {
      const row = [];
      const sizes = [];
      let columnIndex = -1;
      if (table[rowIndex].length > mostCellsPerRow) {
        mostCellsPerRow = table[rowIndex].length;
      }
      while (++columnIndex < table[rowIndex].length) {
        const cell = serialize(table[rowIndex][columnIndex]);
        if (settings.alignDelimiters !== false) {
          const size = stringLength(cell);
          sizes[columnIndex] = size;
          if (longestCellByColumn[columnIndex] === undefined || size > longestCellByColumn[columnIndex]) {
            longestCellByColumn[columnIndex] = size;
          }
        }
        row.push(cell);
      }
      cellMatrix[rowIndex] = row;
      sizeMatrix[rowIndex] = sizes;
    }
    let columnIndex = -1;
    if (typeof align === "object" && "length" in align) {
      while (++columnIndex < mostCellsPerRow) {
        alignments[columnIndex] = toAlignment(align[columnIndex]);
      }
    } else {
      const code = toAlignment(align);
      while (++columnIndex < mostCellsPerRow) {
        alignments[columnIndex] = code;
      }
    }
    columnIndex = -1;
    const row = [];
    const sizes = [];
    while (++columnIndex < mostCellsPerRow) {
      const code = alignments[columnIndex];
      let before = "";
      let after = "";
      if (code === 99) {
        before = ":";
        after = ":";
      } else if (code === 108) {
        before = ":";
      } else if (code === 114) {
        after = ":";
      }
      let size = settings.alignDelimiters === false ? 1 : Math.max(1, longestCellByColumn[columnIndex] - before.length - after.length);
      const cell = before + "-".repeat(size) + after;
      if (settings.alignDelimiters !== false) {
        size = before.length + size + after.length;
        if (size > longestCellByColumn[columnIndex]) {
          longestCellByColumn[columnIndex] = size;
        }
        sizes[columnIndex] = size;
      }
      row[columnIndex] = cell;
    }
    cellMatrix.splice(1, 0, row);
    sizeMatrix.splice(1, 0, sizes);
    rowIndex = -1;
    const lines = [];
    while (++rowIndex < cellMatrix.length) {
      const row = cellMatrix[rowIndex];
      const sizes = sizeMatrix[rowIndex];
      columnIndex = -1;
      const line = [];
      while (++columnIndex < mostCellsPerRow) {
        const cell = row[columnIndex] || "";
        let before = "";
        let after = "";
        if (settings.alignDelimiters !== false) {
          const size = longestCellByColumn[columnIndex] - (sizes[columnIndex] || 0);
          const code = alignments[columnIndex];
          if (code === 114) {
            before = " ".repeat(size);
          } else if (code === 99) {
            if (size % 2) {
              before = " ".repeat(size / 2 + 0.5);
              after = " ".repeat(size / 2 - 0.5);
            } else {
              before = " ".repeat(size / 2);
              after = before;
            }
          } else {
            after = " ".repeat(size);
          }
        }
        if (settings.delimiterStart !== false && !columnIndex) {
          line.push("|");
        }
        if (settings.padding !== false && !(settings.alignDelimiters === false && cell === "") && (settings.delimiterStart !== false || columnIndex)) {
          line.push(" ");
        }
        if (settings.alignDelimiters !== false) {
          line.push(before);
        }
        line.push(cell);
        if (settings.alignDelimiters !== false) {
          line.push(after);
        }
        if (settings.padding !== false) {
          line.push(" ");
        }
        if (settings.delimiterEnd !== false || columnIndex !== mostCellsPerRow - 1) {
          line.push("|");
        }
      }
      lines.push(settings.delimiterEnd === false ? line.join("").replace(/ +$/, "") : line.join(""));
    }
    return lines.join(`
`);
  }
  function serialize(value) {
    return value === null || value === undefined ? "" : String(value);
  }
  function toAlignment(value) {
    const code = typeof value === "string" ? value.codePointAt(0) : 0;
    return code === 67 || code === 99 ? 99 : code === 76 || code === 108 ? 108 : code === 82 || code === 114 ? 114 : 0;
  }
  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/blockquote.js
  function blockquote(node, _, state, info) {
    const exit = state.enter("blockquote");
    const tracker = state.createTracker(info);
    tracker.move("> ");
    tracker.shift(2);
    const value = state.indentLines(state.containerFlow(node, tracker.current()), map);
    exit();
    return value;
  }
  function map(line, _, blank) {
    return ">" + (blank ? "" : " ") + line;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/pattern-in-scope.js
  function patternInScope(stack, pattern) {
    return listInScope(stack, pattern.inConstruct, true) && !listInScope(stack, pattern.notInConstruct, false);
  }
  function listInScope(stack, list, none) {
    if (typeof list === "string") {
      list = [list];
    }
    if (!list || list.length === 0) {
      return none;
    }
    let index = -1;
    while (++index < list.length) {
      if (stack.includes(list[index])) {
        return true;
      }
    }
    return false;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/break.js
  function hardBreak(_, _1, state, info) {
    let index = -1;
    while (++index < state.unsafe.length) {
      if (state.unsafe[index].character === `
` && patternInScope(state.stack, state.unsafe[index])) {
        return /[ \t]/.test(info.before) ? "" : " ";
      }
    }
    return "\\\n";
  }

  // ../../node_modules/.bun/longest-streak@3.1.0/node_modules/longest-streak/index.js
  function longestStreak(value, substring) {
    const source = String(value);
    let index = source.indexOf(substring);
    let expected = index;
    let count = 0;
    let max = 0;
    if (typeof substring !== "string") {
      throw new TypeError("Expected substring");
    }
    while (index !== -1) {
      if (index === expected) {
        if (++count > max) {
          max = count;
        }
      } else {
        count = 1;
      }
      expected = index + substring.length;
      index = source.indexOf(substring, expected);
    }
    return max;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/format-code-as-indented.js
  function formatCodeAsIndented(node, state) {
    return Boolean(state.options.fences === false && node.value && !node.lang && /[^ \r\n]/.test(node.value) && !/^[\t ]*(?:[\r\n]|$)|(?:^|[\r\n])[\t ]*$/.test(node.value));
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/check-fence.js
  function checkFence(state) {
    const marker = state.options.fence || "`";
    if (marker !== "`" && marker !== "~") {
      throw new Error("Cannot serialize code with `" + marker + "` for `options.fence`, expected `` ` `` or `~`");
    }
    return marker;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/code.js
  function code(node, _, state, info) {
    const marker = checkFence(state);
    const raw = node.value || "";
    const suffix = marker === "`" ? "GraveAccent" : "Tilde";
    if (formatCodeAsIndented(node, state)) {
      const exit = state.enter("codeIndented");
      const value = state.indentLines(raw, map2);
      exit();
      return value;
    }
    const tracker = state.createTracker(info);
    const sequence = marker.repeat(Math.max(longestStreak(raw, marker) + 1, 3));
    const exit = state.enter("codeFenced");
    let value = tracker.move(sequence);
    if (node.lang) {
      const subexit = state.enter(`codeFencedLang${suffix}`);
      value += tracker.move(state.safe(node.lang, {
        before: value,
        after: " ",
        encode: ["`"],
        ...tracker.current()
      }));
      subexit();
    }
    if (node.lang && node.meta) {
      const subexit = state.enter(`codeFencedMeta${suffix}`);
      value += tracker.move(" ");
      value += tracker.move(state.safe(node.meta, {
        before: value,
        after: `
`,
        encode: ["`"],
        ...tracker.current()
      }));
      subexit();
    }
    value += tracker.move(`
`);
    if (raw) {
      value += tracker.move(raw + `
`);
    }
    value += tracker.move(sequence);
    exit();
    return value;
  }
  function map2(line, _, blank) {
    return (blank ? "" : "    ") + line;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/check-quote.js
  function checkQuote(state) {
    const marker = state.options.quote || '"';
    if (marker !== '"' && marker !== "'") {
      throw new Error("Cannot serialize title with `" + marker + "` for `options.quote`, expected `\"`, or `'`");
    }
    return marker;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/definition.js
  function definition(node, _, state, info) {
    const quote = checkQuote(state);
    const suffix = quote === '"' ? "Quote" : "Apostrophe";
    const exit = state.enter("definition");
    let subexit = state.enter("label");
    const tracker = state.createTracker(info);
    let value = tracker.move("[");
    value += tracker.move(state.safe(state.associationId(node), {
      before: value,
      after: "]",
      ...tracker.current()
    }));
    value += tracker.move("]: ");
    subexit();
    if (!node.url || /[\0- \u007F]/.test(node.url)) {
      subexit = state.enter("destinationLiteral");
      value += tracker.move("<");
      value += tracker.move(state.safe(node.url, { before: value, after: ">", ...tracker.current() }));
      value += tracker.move(">");
    } else {
      subexit = state.enter("destinationRaw");
      value += tracker.move(state.safe(node.url, {
        before: value,
        after: node.title ? " " : `
`,
        ...tracker.current()
      }));
    }
    subexit();
    if (node.title) {
      subexit = state.enter(`title${suffix}`);
      value += tracker.move(" " + quote);
      value += tracker.move(state.safe(node.title, {
        before: value,
        after: quote,
        ...tracker.current()
      }));
      value += tracker.move(quote);
      subexit();
    }
    exit();
    return value;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/check-emphasis.js
  function checkEmphasis(state) {
    const marker = state.options.emphasis || "*";
    if (marker !== "*" && marker !== "_") {
      throw new Error("Cannot serialize emphasis with `" + marker + "` for `options.emphasis`, expected `*`, or `_`");
    }
    return marker;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/encode-character-reference.js
  function encodeCharacterReference(code) {
    return "&#x" + code.toString(16).toUpperCase() + ";";
  }

  // ../../node_modules/.bun/micromark-util-classify-character@2.0.1/node_modules/micromark-util-classify-character/dev/index.js
  function classifyCharacter(code) {
    if (code === codes.eof || markdownLineEndingOrSpace(code) || unicodeWhitespace(code)) {
      return constants.characterGroupWhitespace;
    }
    if (unicodePunctuation(code)) {
      return constants.characterGroupPunctuation;
    }
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/encode-info.js
  function encodeInfo(outside, inside, marker) {
    const outsideKind = classifyCharacter(outside);
    const insideKind = classifyCharacter(inside);
    if (outsideKind === undefined) {
      return insideKind === undefined ? marker === "_" ? { inside: true, outside: true } : { inside: false, outside: false } : insideKind === 1 ? { inside: true, outside: true } : { inside: false, outside: true };
    }
    if (outsideKind === 1) {
      return insideKind === undefined ? { inside: false, outside: false } : insideKind === 1 ? { inside: true, outside: true } : { inside: false, outside: false };
    }
    return insideKind === undefined ? { inside: false, outside: false } : insideKind === 1 ? { inside: true, outside: false } : { inside: false, outside: false };
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/emphasis.js
  emphasis.peek = emphasisPeek;
  function emphasis(node, _, state, info) {
    const marker = checkEmphasis(state);
    const exit = state.enter("emphasis");
    const tracker = state.createTracker(info);
    const before = tracker.move(marker);
    let between = tracker.move(state.containerPhrasing(node, {
      after: marker,
      before,
      ...tracker.current()
    }));
    const betweenHead = between.charCodeAt(0);
    const open = encodeInfo(info.before.charCodeAt(info.before.length - 1), betweenHead, marker);
    if (open.inside) {
      between = encodeCharacterReference(betweenHead) + between.slice(1);
    }
    const betweenTail = between.charCodeAt(between.length - 1);
    const close = encodeInfo(info.after.charCodeAt(0), betweenTail, marker);
    if (close.inside) {
      between = between.slice(0, -1) + encodeCharacterReference(betweenTail);
    }
    const after = tracker.move(marker);
    exit();
    state.attentionEncodeSurroundingInfo = {
      after: close.outside,
      before: open.outside
    };
    return before + between + after;
  }
  function emphasisPeek(_, _1, state) {
    return state.options.emphasis || "*";
  }

  // ../../node_modules/.bun/unist-util-visit@5.1.0/node_modules/unist-util-visit/lib/index.js
  function visit(tree, testOrVisitor, visitorOrReverse, maybeReverse) {
    let reverse;
    let test;
    let visitor;
    if (typeof testOrVisitor === "function" && typeof visitorOrReverse !== "function") {
      test = undefined;
      visitor = testOrVisitor;
      reverse = visitorOrReverse;
    } else {
      test = testOrVisitor;
      visitor = visitorOrReverse;
      reverse = maybeReverse;
    }
    visitParents(tree, test, overload, reverse);
    function overload(node, parents) {
      const parent = parents[parents.length - 1];
      const index = parent ? parent.children.indexOf(node) : undefined;
      return visitor(node, index, parent);
    }
  }
  // ../../node_modules/.bun/mdast-util-to-string@4.0.0/node_modules/mdast-util-to-string/lib/index.js
  var emptyOptions = {};
  function toString(value, options) {
    const settings = options || emptyOptions;
    const includeImageAlt = typeof settings.includeImageAlt === "boolean" ? settings.includeImageAlt : true;
    const includeHtml = typeof settings.includeHtml === "boolean" ? settings.includeHtml : true;
    return one(value, includeImageAlt, includeHtml);
  }
  function one(value, includeImageAlt, includeHtml) {
    if (node(value)) {
      if ("value" in value) {
        return value.type === "html" && !includeHtml ? "" : value.value;
      }
      if (includeImageAlt && "alt" in value && value.alt) {
        return value.alt;
      }
      if ("children" in value) {
        return all(value.children, includeImageAlt, includeHtml);
      }
    }
    if (Array.isArray(value)) {
      return all(value, includeImageAlt, includeHtml);
    }
    return "";
  }
  function all(values, includeImageAlt, includeHtml) {
    const result = [];
    let index = -1;
    while (++index < values.length) {
      result[index] = one(values[index], includeImageAlt, includeHtml);
    }
    return result.join("");
  }
  function node(value) {
    return Boolean(value && typeof value === "object");
  }
  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/format-heading-as-setext.js
  function formatHeadingAsSetext(node, state) {
    let literalWithBreak = false;
    visit(node, function(node) {
      if ("value" in node && /\r?\n|\r/.test(node.value) || node.type === "break") {
        literalWithBreak = true;
        return EXIT;
      }
    });
    return Boolean((!node.depth || node.depth < 3) && toString(node) && (state.options.setext || literalWithBreak));
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/heading.js
  function heading(node, _, state, info) {
    const rank = Math.max(Math.min(6, node.depth || 1), 1);
    const tracker = state.createTracker(info);
    if (formatHeadingAsSetext(node, state)) {
      const exit = state.enter("headingSetext");
      const subexit = state.enter("phrasing");
      const value = state.containerPhrasing(node, {
        ...tracker.current(),
        before: `
`,
        after: `
`
      });
      subexit();
      exit();
      return value + `
` + (rank === 1 ? "=" : "-").repeat(value.length - (Math.max(value.lastIndexOf("\r"), value.lastIndexOf(`
`)) + 1));
    }
    const sequence = "#".repeat(rank);
    const exit = state.enter("headingAtx");
    const subexit = state.enter("phrasing");
    tracker.move(sequence + " ");
    let value = state.containerPhrasing(node, {
      before: "# ",
      after: `
`,
      ...tracker.current()
    });
    if (/^[\t ]/.test(value)) {
      value = encodeCharacterReference(value.charCodeAt(0)) + value.slice(1);
    }
    value = value ? sequence + " " + value : sequence;
    if (state.options.closeAtx) {
      value += " " + sequence;
    }
    subexit();
    exit();
    return value;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/html.js
  html.peek = htmlPeek;
  function html(node) {
    return node.value || "";
  }
  function htmlPeek() {
    return "<";
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/image.js
  image.peek = imagePeek;
  function image(node, _, state, info) {
    const quote = checkQuote(state);
    const suffix = quote === '"' ? "Quote" : "Apostrophe";
    const exit = state.enter("image");
    let subexit = state.enter("label");
    const tracker = state.createTracker(info);
    let value = tracker.move("![");
    value += tracker.move(state.safe(node.alt, { before: value, after: "]", ...tracker.current() }));
    value += tracker.move("](");
    subexit();
    if (!node.url && node.title || /[\0- \u007F]/.test(node.url)) {
      subexit = state.enter("destinationLiteral");
      value += tracker.move("<");
      value += tracker.move(state.safe(node.url, { before: value, after: ">", ...tracker.current() }));
      value += tracker.move(">");
    } else {
      subexit = state.enter("destinationRaw");
      value += tracker.move(state.safe(node.url, {
        before: value,
        after: node.title ? " " : ")",
        ...tracker.current()
      }));
    }
    subexit();
    if (node.title) {
      subexit = state.enter(`title${suffix}`);
      value += tracker.move(" " + quote);
      value += tracker.move(state.safe(node.title, {
        before: value,
        after: quote,
        ...tracker.current()
      }));
      value += tracker.move(quote);
      subexit();
    }
    value += tracker.move(")");
    exit();
    return value;
  }
  function imagePeek() {
    return "!";
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/image-reference.js
  imageReference.peek = imageReferencePeek;
  function imageReference(node, _, state, info) {
    const type = node.referenceType;
    const exit = state.enter("imageReference");
    let subexit = state.enter("label");
    const tracker = state.createTracker(info);
    let value = tracker.move("![");
    const alt = state.safe(node.alt, {
      before: value,
      after: "]",
      ...tracker.current()
    });
    value += tracker.move(alt + "][");
    subexit();
    const stack = state.stack;
    state.stack = [];
    subexit = state.enter("reference");
    const reference = state.safe(state.associationId(node), {
      before: value,
      after: "]",
      ...tracker.current()
    });
    subexit();
    state.stack = stack;
    exit();
    if (type === "full" || !alt || alt !== reference) {
      value += tracker.move(reference + "]");
    } else if (type === "shortcut") {
      value = value.slice(0, -1);
    } else {
      value += tracker.move("]");
    }
    return value;
  }
  function imageReferencePeek() {
    return "!";
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/inline-code.js
  inlineCode.peek = inlineCodePeek;
  function inlineCode(node, _, state) {
    let value = node.value || "";
    let sequence = "`";
    let index = -1;
    while (new RegExp("(^|[^`])" + sequence + "([^`]|$)").test(value)) {
      sequence += "`";
    }
    if (/[^ \r\n]/.test(value) && (/^[ \r\n]/.test(value) && /[ \r\n]$/.test(value) || /^`|`$/.test(value))) {
      value = " " + value + " ";
    }
    while (++index < state.unsafe.length) {
      const pattern = state.unsafe[index];
      const expression = state.compilePattern(pattern);
      let match;
      if (!pattern.atBreak)
        continue;
      while (match = expression.exec(value)) {
        let position = match.index;
        if (value.charCodeAt(position) === 10 && value.charCodeAt(position - 1) === 13) {
          position--;
        }
        value = value.slice(0, position) + " " + value.slice(match.index + 1);
      }
    }
    return sequence + value + sequence;
  }
  function inlineCodePeek() {
    return "`";
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/format-link-as-autolink.js
  function formatLinkAsAutolink(node, state) {
    const raw = toString(node);
    return Boolean(!state.options.resourceLink && node.url && !node.title && node.children && node.children.length === 1 && node.children[0].type === "text" && (raw === node.url || "mailto:" + raw === node.url) && /^[a-z][a-z+.-]+:/i.test(node.url) && !/[\0- <>\u007F]/.test(node.url));
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/link.js
  link.peek = linkPeek;
  function link(node, _, state, info) {
    const quote = checkQuote(state);
    const suffix = quote === '"' ? "Quote" : "Apostrophe";
    const tracker = state.createTracker(info);
    let exit;
    let subexit;
    if (formatLinkAsAutolink(node, state)) {
      const stack = state.stack;
      state.stack = [];
      exit = state.enter("autolink");
      let value = tracker.move("<");
      value += tracker.move(state.containerPhrasing(node, {
        before: value,
        after: ">",
        ...tracker.current()
      }));
      value += tracker.move(">");
      exit();
      state.stack = stack;
      return value;
    }
    exit = state.enter("link");
    subexit = state.enter("label");
    let value = tracker.move("[");
    value += tracker.move(state.containerPhrasing(node, {
      before: value,
      after: "](",
      ...tracker.current()
    }));
    value += tracker.move("](");
    subexit();
    if (!node.url && node.title || /[\0- \u007F]/.test(node.url)) {
      subexit = state.enter("destinationLiteral");
      value += tracker.move("<");
      value += tracker.move(state.safe(node.url, { before: value, after: ">", ...tracker.current() }));
      value += tracker.move(">");
    } else {
      subexit = state.enter("destinationRaw");
      value += tracker.move(state.safe(node.url, {
        before: value,
        after: node.title ? " " : ")",
        ...tracker.current()
      }));
    }
    subexit();
    if (node.title) {
      subexit = state.enter(`title${suffix}`);
      value += tracker.move(" " + quote);
      value += tracker.move(state.safe(node.title, {
        before: value,
        after: quote,
        ...tracker.current()
      }));
      value += tracker.move(quote);
      subexit();
    }
    value += tracker.move(")");
    exit();
    return value;
  }
  function linkPeek(node, _, state) {
    return formatLinkAsAutolink(node, state) ? "<" : "[";
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/link-reference.js
  linkReference.peek = linkReferencePeek;
  function linkReference(node, _, state, info) {
    const type = node.referenceType;
    const exit = state.enter("linkReference");
    let subexit = state.enter("label");
    const tracker = state.createTracker(info);
    let value = tracker.move("[");
    const text = state.containerPhrasing(node, {
      before: value,
      after: "]",
      ...tracker.current()
    });
    value += tracker.move(text + "][");
    subexit();
    const stack = state.stack;
    state.stack = [];
    subexit = state.enter("reference");
    const reference = state.safe(state.associationId(node), {
      before: value,
      after: "]",
      ...tracker.current()
    });
    subexit();
    state.stack = stack;
    exit();
    if (type === "full" || !text || text !== reference) {
      value += tracker.move(reference + "]");
    } else if (type === "shortcut") {
      value = value.slice(0, -1);
    } else {
      value += tracker.move("]");
    }
    return value;
  }
  function linkReferencePeek() {
    return "[";
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/check-bullet.js
  function checkBullet(state) {
    const marker = state.options.bullet || "*";
    if (marker !== "*" && marker !== "+" && marker !== "-") {
      throw new Error("Cannot serialize items with `" + marker + "` for `options.bullet`, expected `*`, `+`, or `-`");
    }
    return marker;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/check-bullet-other.js
  function checkBulletOther(state) {
    const bullet = checkBullet(state);
    const bulletOther = state.options.bulletOther;
    if (!bulletOther) {
      return bullet === "*" ? "-" : "*";
    }
    if (bulletOther !== "*" && bulletOther !== "+" && bulletOther !== "-") {
      throw new Error("Cannot serialize items with `" + bulletOther + "` for `options.bulletOther`, expected `*`, `+`, or `-`");
    }
    if (bulletOther === bullet) {
      throw new Error("Expected `bullet` (`" + bullet + "`) and `bulletOther` (`" + bulletOther + "`) to be different");
    }
    return bulletOther;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/check-bullet-ordered.js
  function checkBulletOrdered(state) {
    const marker = state.options.bulletOrdered || ".";
    if (marker !== "." && marker !== ")") {
      throw new Error("Cannot serialize items with `" + marker + "` for `options.bulletOrdered`, expected `.` or `)`");
    }
    return marker;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/check-rule.js
  function checkRule(state) {
    const marker = state.options.rule || "*";
    if (marker !== "*" && marker !== "-" && marker !== "_") {
      throw new Error("Cannot serialize rules with `" + marker + "` for `options.rule`, expected `*`, `-`, or `_`");
    }
    return marker;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/list.js
  function list(node, parent, state, info) {
    const exit = state.enter("list");
    const bulletCurrent = state.bulletCurrent;
    let bullet = node.ordered ? checkBulletOrdered(state) : checkBullet(state);
    const bulletOther = node.ordered ? bullet === "." ? ")" : "." : checkBulletOther(state);
    let useDifferentMarker = parent && state.bulletLastUsed ? bullet === state.bulletLastUsed : false;
    if (!node.ordered) {
      const firstListItem = node.children ? node.children[0] : undefined;
      if ((bullet === "*" || bullet === "-") && firstListItem && (!firstListItem.children || !firstListItem.children[0]) && state.stack[state.stack.length - 1] === "list" && state.stack[state.stack.length - 2] === "listItem" && state.stack[state.stack.length - 3] === "list" && state.stack[state.stack.length - 4] === "listItem" && state.indexStack[state.indexStack.length - 1] === 0 && state.indexStack[state.indexStack.length - 2] === 0 && state.indexStack[state.indexStack.length - 3] === 0) {
        useDifferentMarker = true;
      }
      if (checkRule(state) === bullet && firstListItem) {
        let index = -1;
        while (++index < node.children.length) {
          const item = node.children[index];
          if (item && item.type === "listItem" && item.children && item.children[0] && item.children[0].type === "thematicBreak") {
            useDifferentMarker = true;
            break;
          }
        }
      }
    }
    if (useDifferentMarker) {
      bullet = bulletOther;
    }
    state.bulletCurrent = bullet;
    const value = state.containerFlow(node, info);
    state.bulletLastUsed = bullet;
    state.bulletCurrent = bulletCurrent;
    exit();
    return value;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/check-list-item-indent.js
  function checkListItemIndent(state) {
    const style = state.options.listItemIndent || "one";
    if (style !== "tab" && style !== "one" && style !== "mixed") {
      throw new Error("Cannot serialize items with `" + style + "` for `options.listItemIndent`, expected `tab`, `one`, or `mixed`");
    }
    return style;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/list-item.js
  function listItem(node, parent, state, info) {
    const listItemIndent = checkListItemIndent(state);
    let bullet = state.bulletCurrent || checkBullet(state);
    if (parent && parent.type === "list" && parent.ordered) {
      bullet = (typeof parent.start === "number" && parent.start > -1 ? parent.start : 1) + (state.options.incrementListMarker === false ? 0 : parent.children.indexOf(node)) + bullet;
    }
    let size = bullet.length + 1;
    if (listItemIndent === "tab" || listItemIndent === "mixed" && (parent && parent.type === "list" && parent.spread || node.spread)) {
      size = Math.ceil(size / 4) * 4;
    }
    const tracker = state.createTracker(info);
    tracker.move(bullet + " ".repeat(size - bullet.length));
    tracker.shift(size);
    const exit = state.enter("listItem");
    const value = state.indentLines(state.containerFlow(node, tracker.current()), map);
    exit();
    return value;
    function map(line, index, blank) {
      if (index) {
        return (blank ? "" : " ".repeat(size)) + line;
      }
      return (blank ? bullet : bullet + " ".repeat(size - bullet.length)) + line;
    }
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/paragraph.js
  function paragraph(node, _, state, info) {
    const exit = state.enter("paragraph");
    const subexit = state.enter("phrasing");
    const value = state.containerPhrasing(node, info);
    subexit();
    exit();
    return value;
  }

  // ../../node_modules/.bun/mdast-util-phrasing@4.1.0/node_modules/mdast-util-phrasing/lib/index.js
  var phrasing = convert([
    "break",
    "delete",
    "emphasis",
    "footnote",
    "footnoteReference",
    "image",
    "imageReference",
    "inlineCode",
    "inlineMath",
    "link",
    "linkReference",
    "mdxJsxTextElement",
    "mdxTextExpression",
    "strong",
    "text",
    "textDirective"
  ]);
  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/root.js
  function root(node, _, state, info) {
    const hasPhrasing = node.children.some(function(d) {
      return phrasing(d);
    });
    const container = hasPhrasing ? state.containerPhrasing : state.containerFlow;
    return container.call(state, node, info);
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/check-strong.js
  function checkStrong(state) {
    const marker = state.options.strong || "*";
    if (marker !== "*" && marker !== "_") {
      throw new Error("Cannot serialize strong with `" + marker + "` for `options.strong`, expected `*`, or `_`");
    }
    return marker;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/strong.js
  strong.peek = strongPeek;
  function strong(node, _, state, info) {
    const marker = checkStrong(state);
    const exit = state.enter("strong");
    const tracker = state.createTracker(info);
    const before = tracker.move(marker + marker);
    let between = tracker.move(state.containerPhrasing(node, {
      after: marker,
      before,
      ...tracker.current()
    }));
    const betweenHead = between.charCodeAt(0);
    const open = encodeInfo(info.before.charCodeAt(info.before.length - 1), betweenHead, marker);
    if (open.inside) {
      between = encodeCharacterReference(betweenHead) + between.slice(1);
    }
    const betweenTail = between.charCodeAt(between.length - 1);
    const close = encodeInfo(info.after.charCodeAt(0), betweenTail, marker);
    if (close.inside) {
      between = between.slice(0, -1) + encodeCharacterReference(betweenTail);
    }
    const after = tracker.move(marker + marker);
    exit();
    state.attentionEncodeSurroundingInfo = {
      after: close.outside,
      before: open.outside
    };
    return before + between + after;
  }
  function strongPeek(_, _1, state) {
    return state.options.strong || "*";
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/text.js
  function text(node, _, state, info) {
    return state.safe(node.value, info);
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/util/check-rule-repetition.js
  function checkRuleRepetition(state) {
    const repetition = state.options.ruleRepetition || 3;
    if (repetition < 3) {
      throw new Error("Cannot serialize rules with repetition `" + repetition + "` for `options.ruleRepetition`, expected `3` or more");
    }
    return repetition;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/thematic-break.js
  function thematicBreak(_, _1, state) {
    const value = (checkRule(state) + (state.options.ruleSpaces ? " " : "")).repeat(checkRuleRepetition(state));
    return state.options.ruleSpaces ? value.slice(0, -1) : value;
  }

  // ../../node_modules/.bun/mdast-util-to-markdown@2.1.2/node_modules/mdast-util-to-markdown/lib/handle/index.js
  var handle = {
    blockquote,
    break: hardBreak,
    code,
    definition,
    emphasis,
    hardBreak,
    heading,
    html,
    image,
    imageReference,
    inlineCode,
    link,
    linkReference,
    list,
    listItem,
    paragraph,
    root,
    strong,
    text,
    thematicBreak
  };
  // ../../node_modules/.bun/mdast-util-gfm-table@2.0.0/node_modules/mdast-util-gfm-table/lib/index.js
  function gfmTableFromMarkdown() {
    return {
      enter: {
        table: enterTable,
        tableData: enterCell,
        tableHeader: enterCell,
        tableRow: enterRow
      },
      exit: {
        codeText: exitCodeText,
        table: exitTable,
        tableData: exit,
        tableHeader: exit,
        tableRow: exit
      }
    };
  }
  function enterTable(token) {
    const align = token._align;
    ok(align, "expected `_align` on table");
    this.enter({
      type: "table",
      align: align.map(function(d) {
        return d === "none" ? null : d;
      }),
      children: []
    }, token);
    this.data.inTable = true;
  }
  function exitTable(token) {
    this.exit(token);
    this.data.inTable = undefined;
  }
  function enterRow(token) {
    this.enter({ type: "tableRow", children: [] }, token);
  }
  function exit(token) {
    this.exit(token);
  }
  function enterCell(token) {
    this.enter({ type: "tableCell", children: [] }, token);
  }
  function exitCodeText(token) {
    let value = this.resume();
    if (this.data.inTable) {
      value = value.replace(/\\([\\|])/g, replace);
    }
    const node = this.stack[this.stack.length - 1];
    ok(node.type === "inlineCode");
    node.value = value;
    this.exit(token);
  }
  function replace($0, $1) {
    return $1 === "|" ? $1 : $0;
  }
  function gfmTableToMarkdown(options) {
    const settings = options || {};
    const padding = settings.tableCellPadding;
    const alignDelimiters = settings.tablePipeAlign;
    const stringLength = settings.stringLength;
    const around = padding ? " " : "|";
    return {
      unsafe: [
        { character: "\r", inConstruct: "tableCell" },
        { character: `
`, inConstruct: "tableCell" },
        { atBreak: true, character: "|", after: "[\t :-]" },
        { character: "|", inConstruct: "tableCell" },
        { atBreak: true, character: ":", after: "-" },
        { atBreak: true, character: "-", after: "[:|-]" }
      ],
      handlers: {
        inlineCode: inlineCodeWithTable,
        table: handleTable,
        tableCell: handleTableCell,
        tableRow: handleTableRow
      }
    };
    function handleTable(node, _, state, info) {
      return serializeData(handleTableAsData(node, state, info), node.align);
    }
    function handleTableRow(node, _, state, info) {
      const row = handleTableRowAsData(node, state, info);
      const value = serializeData([row]);
      return value.slice(0, value.indexOf(`
`));
    }
    function handleTableCell(node, _, state, info) {
      const exit = state.enter("tableCell");
      const subexit = state.enter("phrasing");
      const value = state.containerPhrasing(node, {
        ...info,
        before: around,
        after: around
      });
      subexit();
      exit();
      return value;
    }
    function serializeData(matrix, align) {
      return markdownTable(matrix, {
        align,
        alignDelimiters,
        padding,
        stringLength
      });
    }
    function handleTableAsData(node, state, info) {
      const children = node.children;
      let index = -1;
      const result = [];
      const subexit = state.enter("table");
      while (++index < children.length) {
        result[index] = handleTableRowAsData(children[index], state, info);
      }
      subexit();
      return result;
    }
    function handleTableRowAsData(node, state, info) {
      const children = node.children;
      let index = -1;
      const result = [];
      const subexit = state.enter("tableRow");
      while (++index < children.length) {
        result[index] = handleTableCell(children[index], node, state, info);
      }
      subexit();
      return result;
    }
    function inlineCodeWithTable(node, parent, state) {
      let value = handle.inlineCode(node, parent, state);
      if (state.stack.includes("tableCell")) {
        value = value.replace(/\|/g, "\\$&");
      }
      return value;
    }
  }
  // ../../node_modules/.bun/mdast-util-gfm-task-list-item@2.0.0/node_modules/mdast-util-gfm-task-list-item/lib/index.js
  function gfmTaskListItemFromMarkdown() {
    return {
      exit: {
        taskListCheckValueChecked: exitCheck,
        taskListCheckValueUnchecked: exitCheck,
        paragraph: exitParagraphWithTaskListItem
      }
    };
  }
  function gfmTaskListItemToMarkdown() {
    return {
      unsafe: [{ atBreak: true, character: "-", after: "[:|-]" }],
      handlers: { listItem: listItemWithTaskListItem }
    };
  }
  function exitCheck(token) {
    const node = this.stack[this.stack.length - 2];
    ok(node.type === "listItem");
    node.checked = token.type === "taskListCheckValueChecked";
  }
  function exitParagraphWithTaskListItem(token) {
    const parent = this.stack[this.stack.length - 2];
    if (parent && parent.type === "listItem" && typeof parent.checked === "boolean") {
      const node = this.stack[this.stack.length - 1];
      ok(node.type === "paragraph");
      const head = node.children[0];
      if (head && head.type === "text") {
        const siblings = parent.children;
        let index = -1;
        let firstParaghraph;
        while (++index < siblings.length) {
          const sibling = siblings[index];
          if (sibling.type === "paragraph") {
            firstParaghraph = sibling;
            break;
          }
        }
        if (firstParaghraph === node) {
          head.value = head.value.slice(1);
          if (head.value.length === 0) {
            node.children.shift();
          } else if (node.position && head.position && typeof head.position.start.offset === "number") {
            head.position.start.column++;
            head.position.start.offset++;
            node.position.start = Object.assign({}, head.position.start);
          }
        }
      }
    }
    this.exit(token);
  }
  function listItemWithTaskListItem(node, parent, state, info) {
    const head = node.children[0];
    const checkable = typeof node.checked === "boolean" && head && head.type === "paragraph";
    const checkbox = "[" + (node.checked ? "x" : " ") + "] ";
    const tracker = state.createTracker(info);
    if (checkable) {
      tracker.move(checkbox);
    }
    let value = handle.listItem(node, parent, state, {
      ...info,
      ...tracker.current()
    });
    if (checkable) {
      value = value.replace(/^(?:[*+-]|\d+\.)([\r\n]| {1,3})/, check);
    }
    return value;
    function check($0) {
      return $0 + checkbox;
    }
  }
  // ../../node_modules/.bun/mdast-util-gfm@3.1.0/node_modules/mdast-util-gfm/lib/index.js
  function gfmFromMarkdown() {
    return [
      gfmAutolinkLiteralFromMarkdown(),
      gfmFootnoteFromMarkdown(),
      gfmStrikethroughFromMarkdown(),
      gfmTableFromMarkdown(),
      gfmTaskListItemFromMarkdown()
    ];
  }
  function gfmToMarkdown(options) {
    return {
      extensions: [
        gfmAutolinkLiteralToMarkdown(),
        gfmFootnoteToMarkdown(options),
        gfmStrikethroughToMarkdown(),
        gfmTableToMarkdown(options),
        gfmTaskListItemToMarkdown()
      ]
    };
  }
  // ../../node_modules/.bun/micromark-util-chunked@2.0.1/node_modules/micromark-util-chunked/dev/index.js
  function splice(list, start, remove, items) {
    const end = list.length;
    let chunkStart = 0;
    let parameters;
    if (start < 0) {
      start = -start > end ? 0 : end + start;
    } else {
      start = start > end ? end : start;
    }
    remove = remove > 0 ? remove : 0;
    if (items.length < constants.v8MaxSafeChunkSize) {
      parameters = Array.from(items);
      parameters.unshift(start, remove);
      list.splice(...parameters);
    } else {
      if (remove)
        list.splice(start, remove);
      while (chunkStart < items.length) {
        parameters = items.slice(chunkStart, chunkStart + constants.v8MaxSafeChunkSize);
        parameters.unshift(start, 0);
        list.splice(...parameters);
        chunkStart += constants.v8MaxSafeChunkSize;
        start += constants.v8MaxSafeChunkSize;
      }
    }
  }
  function push(list, items) {
    if (list.length > 0) {
      splice(list, list.length, 0, items);
      return list;
    }
    return items;
  }

  // ../../node_modules/.bun/micromark-util-combine-extensions@2.0.1/node_modules/micromark-util-combine-extensions/index.js
  var hasOwnProperty = {}.hasOwnProperty;
  function combineExtensions(extensions) {
    const all = {};
    let index = -1;
    while (++index < extensions.length) {
      syntaxExtension(all, extensions[index]);
    }
    return all;
  }
  function syntaxExtension(all, extension) {
    let hook;
    for (hook in extension) {
      const maybe = hasOwnProperty.call(all, hook) ? all[hook] : undefined;
      const left = maybe || (all[hook] = {});
      const right = extension[hook];
      let code;
      if (right) {
        for (code in right) {
          if (!hasOwnProperty.call(left, code))
            left[code] = [];
          const value = right[code];
          constructs(left[code], Array.isArray(value) ? value : value ? [value] : []);
        }
      }
    }
  }
  function constructs(existing, list) {
    let index = -1;
    const before = [];
    while (++index < list.length) {
      (list[index].add === "after" ? existing : before).push(list[index]);
    }
    splice(existing, 0, 0, before);
  }

  // ../../node_modules/.bun/micromark-extension-gfm-autolink-literal@2.1.0/node_modules/micromark-extension-gfm-autolink-literal/dev/lib/syntax.js
  var wwwPrefix = { tokenize: tokenizeWwwPrefix, partial: true };
  var domain = { tokenize: tokenizeDomain, partial: true };
  var path = { tokenize: tokenizePath, partial: true };
  var trail = { tokenize: tokenizeTrail, partial: true };
  var emailDomainDotTrail = {
    tokenize: tokenizeEmailDomainDotTrail,
    partial: true
  };
  var wwwAutolink = {
    name: "wwwAutolink",
    tokenize: tokenizeWwwAutolink,
    previous: previousWww
  };
  var protocolAutolink = {
    name: "protocolAutolink",
    tokenize: tokenizeProtocolAutolink,
    previous: previousProtocol
  };
  var emailAutolink = {
    name: "emailAutolink",
    tokenize: tokenizeEmailAutolink,
    previous: previousEmail
  };
  var text2 = {};
  function gfmAutolinkLiteral() {
    return { text: text2 };
  }
  var code2 = codes.digit0;
  while (code2 < codes.leftCurlyBrace) {
    text2[code2] = emailAutolink;
    code2++;
    if (code2 === codes.colon)
      code2 = codes.uppercaseA;
    else if (code2 === codes.leftSquareBracket)
      code2 = codes.lowercaseA;
  }
  text2[codes.plusSign] = emailAutolink;
  text2[codes.dash] = emailAutolink;
  text2[codes.dot] = emailAutolink;
  text2[codes.underscore] = emailAutolink;
  text2[codes.uppercaseH] = [emailAutolink, protocolAutolink];
  text2[codes.lowercaseH] = [emailAutolink, protocolAutolink];
  text2[codes.uppercaseW] = [emailAutolink, wwwAutolink];
  text2[codes.lowercaseW] = [emailAutolink, wwwAutolink];
  function tokenizeEmailAutolink(effects, ok, nok) {
    const self2 = this;
    let dot;
    let data;
    return start;
    function start(code) {
      if (!gfmAtext(code) || !previousEmail.call(self2, self2.previous) || previousUnbalanced(self2.events)) {
        return nok(code);
      }
      effects.enter("literalAutolink");
      effects.enter("literalAutolinkEmail");
      return atext(code);
    }
    function atext(code) {
      if (gfmAtext(code)) {
        effects.consume(code);
        return atext;
      }
      if (code === codes.atSign) {
        effects.consume(code);
        return emailDomain;
      }
      return nok(code);
    }
    function emailDomain(code) {
      if (code === codes.dot) {
        return effects.check(emailDomainDotTrail, emailDomainAfter, emailDomainDot)(code);
      }
      if (code === codes.dash || code === codes.underscore || asciiAlphanumeric(code)) {
        data = true;
        effects.consume(code);
        return emailDomain;
      }
      return emailDomainAfter(code);
    }
    function emailDomainDot(code) {
      effects.consume(code);
      dot = true;
      return emailDomain;
    }
    function emailDomainAfter(code) {
      if (data && dot && asciiAlpha(self2.previous)) {
        effects.exit("literalAutolinkEmail");
        effects.exit("literalAutolink");
        return ok(code);
      }
      return nok(code);
    }
  }
  function tokenizeWwwAutolink(effects, ok, nok) {
    const self2 = this;
    return wwwStart;
    function wwwStart(code) {
      if (code !== codes.uppercaseW && code !== codes.lowercaseW || !previousWww.call(self2, self2.previous) || previousUnbalanced(self2.events)) {
        return nok(code);
      }
      effects.enter("literalAutolink");
      effects.enter("literalAutolinkWww");
      return effects.check(wwwPrefix, effects.attempt(domain, effects.attempt(path, wwwAfter), nok), nok)(code);
    }
    function wwwAfter(code) {
      effects.exit("literalAutolinkWww");
      effects.exit("literalAutolink");
      return ok(code);
    }
  }
  function tokenizeProtocolAutolink(effects, ok, nok) {
    const self2 = this;
    let buffer = "";
    let seen = false;
    return protocolStart;
    function protocolStart(code) {
      if ((code === codes.uppercaseH || code === codes.lowercaseH) && previousProtocol.call(self2, self2.previous) && !previousUnbalanced(self2.events)) {
        effects.enter("literalAutolink");
        effects.enter("literalAutolinkHttp");
        buffer += String.fromCodePoint(code);
        effects.consume(code);
        return protocolPrefixInside;
      }
      return nok(code);
    }
    function protocolPrefixInside(code) {
      if (asciiAlpha(code) && buffer.length < 5) {
        buffer += String.fromCodePoint(code);
        effects.consume(code);
        return protocolPrefixInside;
      }
      if (code === codes.colon) {
        const protocol = buffer.toLowerCase();
        if (protocol === "http" || protocol === "https") {
          effects.consume(code);
          return protocolSlashesInside;
        }
      }
      return nok(code);
    }
    function protocolSlashesInside(code) {
      if (code === codes.slash) {
        effects.consume(code);
        if (seen) {
          return afterProtocol;
        }
        seen = true;
        return protocolSlashesInside;
      }
      return nok(code);
    }
    function afterProtocol(code) {
      return code === codes.eof || asciiControl(code) || markdownLineEndingOrSpace(code) || unicodeWhitespace(code) || unicodePunctuation(code) ? nok(code) : effects.attempt(domain, effects.attempt(path, protocolAfter), nok)(code);
    }
    function protocolAfter(code) {
      effects.exit("literalAutolinkHttp");
      effects.exit("literalAutolink");
      return ok(code);
    }
  }
  function tokenizeWwwPrefix(effects, ok, nok) {
    let size = 0;
    return wwwPrefixInside;
    function wwwPrefixInside(code) {
      if ((code === codes.uppercaseW || code === codes.lowercaseW) && size < 3) {
        size++;
        effects.consume(code);
        return wwwPrefixInside;
      }
      if (code === codes.dot && size === 3) {
        effects.consume(code);
        return wwwPrefixAfter;
      }
      return nok(code);
    }
    function wwwPrefixAfter(code) {
      return code === codes.eof ? nok(code) : ok(code);
    }
  }
  function tokenizeDomain(effects, ok, nok) {
    let underscoreInLastSegment;
    let underscoreInLastLastSegment;
    let seen;
    return domainInside;
    function domainInside(code) {
      if (code === codes.dot || code === codes.underscore) {
        return effects.check(trail, domainAfter, domainAtPunctuation)(code);
      }
      if (code === codes.eof || markdownLineEndingOrSpace(code) || unicodeWhitespace(code) || code !== codes.dash && unicodePunctuation(code)) {
        return domainAfter(code);
      }
      seen = true;
      effects.consume(code);
      return domainInside;
    }
    function domainAtPunctuation(code) {
      if (code === codes.underscore) {
        underscoreInLastSegment = true;
      } else {
        underscoreInLastLastSegment = underscoreInLastSegment;
        underscoreInLastSegment = undefined;
      }
      effects.consume(code);
      return domainInside;
    }
    function domainAfter(code) {
      if (underscoreInLastLastSegment || underscoreInLastSegment || !seen) {
        return nok(code);
      }
      return ok(code);
    }
  }
  function tokenizePath(effects, ok) {
    let sizeOpen = 0;
    let sizeClose = 0;
    return pathInside;
    function pathInside(code) {
      if (code === codes.leftParenthesis) {
        sizeOpen++;
        effects.consume(code);
        return pathInside;
      }
      if (code === codes.rightParenthesis && sizeClose < sizeOpen) {
        return pathAtPunctuation(code);
      }
      if (code === codes.exclamationMark || code === codes.quotationMark || code === codes.ampersand || code === codes.apostrophe || code === codes.rightParenthesis || code === codes.asterisk || code === codes.comma || code === codes.dot || code === codes.colon || code === codes.semicolon || code === codes.lessThan || code === codes.questionMark || code === codes.rightSquareBracket || code === codes.underscore || code === codes.tilde) {
        return effects.check(trail, ok, pathAtPunctuation)(code);
      }
      if (code === codes.eof || markdownLineEndingOrSpace(code) || unicodeWhitespace(code)) {
        return ok(code);
      }
      effects.consume(code);
      return pathInside;
    }
    function pathAtPunctuation(code) {
      if (code === codes.rightParenthesis) {
        sizeClose++;
      }
      effects.consume(code);
      return pathInside;
    }
  }
  function tokenizeTrail(effects, ok, nok) {
    return trail;
    function trail(code) {
      if (code === codes.exclamationMark || code === codes.quotationMark || code === codes.apostrophe || code === codes.rightParenthesis || code === codes.asterisk || code === codes.comma || code === codes.dot || code === codes.colon || code === codes.semicolon || code === codes.questionMark || code === codes.underscore || code === codes.tilde) {
        effects.consume(code);
        return trail;
      }
      if (code === codes.ampersand) {
        effects.consume(code);
        return trailCharacterReferenceStart;
      }
      if (code === codes.rightSquareBracket) {
        effects.consume(code);
        return trailBracketAfter;
      }
      if (code === codes.lessThan || code === codes.eof || markdownLineEndingOrSpace(code) || unicodeWhitespace(code)) {
        return ok(code);
      }
      return nok(code);
    }
    function trailBracketAfter(code) {
      if (code === codes.eof || code === codes.leftParenthesis || code === codes.leftSquareBracket || markdownLineEndingOrSpace(code) || unicodeWhitespace(code)) {
        return ok(code);
      }
      return trail(code);
    }
    function trailCharacterReferenceStart(code) {
      return asciiAlpha(code) ? trailCharacterReferenceInside(code) : nok(code);
    }
    function trailCharacterReferenceInside(code) {
      if (code === codes.semicolon) {
        effects.consume(code);
        return trail;
      }
      if (asciiAlpha(code)) {
        effects.consume(code);
        return trailCharacterReferenceInside;
      }
      return nok(code);
    }
  }
  function tokenizeEmailDomainDotTrail(effects, ok, nok) {
    return start;
    function start(code) {
      effects.consume(code);
      return after;
    }
    function after(code) {
      return asciiAlphanumeric(code) ? nok(code) : ok(code);
    }
  }
  function previousWww(code) {
    return code === codes.eof || code === codes.leftParenthesis || code === codes.asterisk || code === codes.underscore || code === codes.leftSquareBracket || code === codes.rightSquareBracket || code === codes.tilde || markdownLineEndingOrSpace(code);
  }
  function previousProtocol(code) {
    return !asciiAlpha(code);
  }
  function previousEmail(code) {
    return !(code === codes.slash || gfmAtext(code));
  }
  function gfmAtext(code) {
    return code === codes.plusSign || code === codes.dash || code === codes.dot || code === codes.underscore || asciiAlphanumeric(code);
  }
  function previousUnbalanced(events) {
    let index = events.length;
    let result = false;
    while (index--) {
      const token = events[index][1];
      if ((token.type === "labelLink" || token.type === "labelImage") && !token._balanced) {
        result = true;
        break;
      }
      if (token._gfmAutolinkLiteralWalkedInto) {
        result = false;
        break;
      }
    }
    if (events.length > 0 && !result) {
      events[events.length - 1][1]._gfmAutolinkLiteralWalkedInto = true;
    }
    return result;
  }
  // ../../node_modules/.bun/micromark-util-sanitize-uri@2.0.1/node_modules/micromark-util-sanitize-uri/dev/index.js
  function normalizeUri(value) {
    const result = [];
    let index = -1;
    let start = 0;
    let skip = 0;
    while (++index < value.length) {
      const code = value.charCodeAt(index);
      let replace = "";
      if (code === codes.percentSign && asciiAlphanumeric(value.charCodeAt(index + 1)) && asciiAlphanumeric(value.charCodeAt(index + 2))) {
        skip = 2;
      } else if (code < 128) {
        if (!/[!#$&-;=?-Z_a-z~]/.test(String.fromCharCode(code))) {
          replace = String.fromCharCode(code);
        }
      } else if (code > 55295 && code < 57344) {
        const next = value.charCodeAt(index + 1);
        if (code < 56320 && next > 56319 && next < 57344) {
          replace = String.fromCharCode(code, next);
          skip = 1;
        } else {
          replace = values.replacementCharacter;
        }
      } else {
        replace = String.fromCharCode(code);
      }
      if (replace) {
        result.push(value.slice(start, index), encodeURIComponent(replace));
        start = index + skip + 1;
        replace = "";
      }
      if (skip) {
        index += skip;
        skip = 0;
      }
    }
    return result.join("") + value.slice(start);
  }

  // ../../node_modules/.bun/micromark-util-resolve-all@2.0.1/node_modules/micromark-util-resolve-all/index.js
  function resolveAll(constructs, events, context) {
    const called = [];
    let index = -1;
    while (++index < constructs.length) {
      const resolve = constructs[index].resolveAll;
      if (resolve && !called.includes(resolve)) {
        events = resolve(events, context);
        called.push(resolve);
      }
    }
    return events;
  }

  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/attention.js
  var attention = {
    name: "attention",
    resolveAll: resolveAllAttention,
    tokenize: tokenizeAttention
  };
  function resolveAllAttention(events, context) {
    let index = -1;
    let nextEvents;
    while (++index < events.length) {
      if (events[index][0] === "enter" && events[index][1].type === "attentionSequence" && events[index][1]._close) {
        let open = index;
        while (open--) {
          if (events[open][0] === "exit" && events[open][1].type === "attentionSequence" && events[open][1]._open && context.sliceSerialize(events[open][1]).charCodeAt(0) === context.sliceSerialize(events[index][1]).charCodeAt(0)) {
            if ((events[open][1]._close || events[index][1]._open) && (events[index][1].end.offset - events[index][1].start.offset) % 3 && !((events[open][1].end.offset - events[open][1].start.offset + events[index][1].end.offset - events[index][1].start.offset) % 3)) {
              continue;
            }
            const use = events[open][1].end.offset - events[open][1].start.offset > 1 && events[index][1].end.offset - events[index][1].start.offset > 1 ? 2 : 1;
            const start = { ...events[open][1].end };
            const end = { ...events[index][1].start };
            movePoint(start, -use);
            movePoint(end, use);
            const openingSequence = {
              type: use > 1 ? types.strongSequence : types.emphasisSequence,
              start,
              end: { ...events[open][1].end }
            };
            const closingSequence = {
              type: use > 1 ? types.strongSequence : types.emphasisSequence,
              start: { ...events[index][1].start },
              end
            };
            const text = {
              type: use > 1 ? types.strongText : types.emphasisText,
              start: { ...events[open][1].end },
              end: { ...events[index][1].start }
            };
            const group = {
              type: use > 1 ? types.strong : types.emphasis,
              start: { ...openingSequence.start },
              end: { ...closingSequence.end }
            };
            events[open][1].end = { ...openingSequence.start };
            events[index][1].start = { ...closingSequence.end };
            nextEvents = [];
            if (events[open][1].end.offset - events[open][1].start.offset) {
              nextEvents = push(nextEvents, [
                ["enter", events[open][1], context],
                ["exit", events[open][1], context]
              ]);
            }
            nextEvents = push(nextEvents, [
              ["enter", group, context],
              ["enter", openingSequence, context],
              ["exit", openingSequence, context],
              ["enter", text, context]
            ]);
            ok(context.parser.constructs.insideSpan.null, "expected `insideSpan` to be populated");
            nextEvents = push(nextEvents, resolveAll(context.parser.constructs.insideSpan.null, events.slice(open + 1, index), context));
            nextEvents = push(nextEvents, [
              ["exit", text, context],
              ["enter", closingSequence, context],
              ["exit", closingSequence, context],
              ["exit", group, context]
            ]);
            let offset = 0;
            if (events[index][1].end.offset - events[index][1].start.offset) {
              offset = 2;
              nextEvents = push(nextEvents, [
                ["enter", events[index][1], context],
                ["exit", events[index][1], context]
              ]);
            }
            splice(events, open - 1, index - open + 3, nextEvents);
            index = open + nextEvents.length - offset - 2;
            break;
          }
        }
      }
    }
    index = -1;
    while (++index < events.length) {
      if (events[index][1].type === "attentionSequence") {
        events[index][1].type = "data";
      }
    }
    return events;
  }
  function tokenizeAttention(effects, ok2) {
    const attentionMarkers = this.parser.constructs.attentionMarkers.null;
    const previous = this.previous;
    const before = classifyCharacter(previous);
    let marker;
    return start;
    function start(code) {
      ok(code === codes.asterisk || code === codes.underscore, "expected asterisk or underscore");
      marker = code;
      effects.enter("attentionSequence");
      return inside(code);
    }
    function inside(code) {
      if (code === marker) {
        effects.consume(code);
        return inside;
      }
      const token = effects.exit("attentionSequence");
      const after = classifyCharacter(code);
      ok(attentionMarkers, "expected `attentionMarkers` to be populated");
      const open = !after || after === constants.characterGroupPunctuation && before || attentionMarkers.includes(code) && code !== codes.asterisk && code !== codes.underscore;
      const close = !before || before === constants.characterGroupPunctuation && after || attentionMarkers.includes(previous) && previous !== codes.asterisk && previous !== codes.underscore;
      token._open = Boolean(marker === codes.asterisk ? open : open && (before || !close));
      token._close = Boolean(marker === codes.asterisk ? close : close && (after || !open));
      return ok2(code);
    }
  }
  function movePoint(point, offset) {
    point.column += offset;
    point.offset += offset;
    point._bufferIndex += offset;
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/autolink.js
  var autolink = { name: "autolink", tokenize: tokenizeAutolink };
  function tokenizeAutolink(effects, ok2, nok) {
    let size = 0;
    return start;
    function start(code) {
      ok(code === codes.lessThan, "expected `<`");
      effects.enter(types.autolink);
      effects.enter(types.autolinkMarker);
      effects.consume(code);
      effects.exit(types.autolinkMarker);
      effects.enter(types.autolinkProtocol);
      return open;
    }
    function open(code) {
      if (asciiAlpha(code)) {
        effects.consume(code);
        return schemeOrEmailAtext;
      }
      if (code === codes.atSign) {
        return nok(code);
      }
      return emailAtext(code);
    }
    function schemeOrEmailAtext(code) {
      if (code === codes.plusSign || code === codes.dash || code === codes.dot || asciiAlphanumeric(code)) {
        size = 1;
        return schemeInsideOrEmailAtext(code);
      }
      return emailAtext(code);
    }
    function schemeInsideOrEmailAtext(code) {
      if (code === codes.colon) {
        effects.consume(code);
        size = 0;
        return urlInside;
      }
      if ((code === codes.plusSign || code === codes.dash || code === codes.dot || asciiAlphanumeric(code)) && size++ < constants.autolinkSchemeSizeMax) {
        effects.consume(code);
        return schemeInsideOrEmailAtext;
      }
      size = 0;
      return emailAtext(code);
    }
    function urlInside(code) {
      if (code === codes.greaterThan) {
        effects.exit(types.autolinkProtocol);
        effects.enter(types.autolinkMarker);
        effects.consume(code);
        effects.exit(types.autolinkMarker);
        effects.exit(types.autolink);
        return ok2;
      }
      if (code === codes.eof || code === codes.space || code === codes.lessThan || asciiControl(code)) {
        return nok(code);
      }
      effects.consume(code);
      return urlInside;
    }
    function emailAtext(code) {
      if (code === codes.atSign) {
        effects.consume(code);
        return emailAtSignOrDot;
      }
      if (asciiAtext(code)) {
        effects.consume(code);
        return emailAtext;
      }
      return nok(code);
    }
    function emailAtSignOrDot(code) {
      return asciiAlphanumeric(code) ? emailLabel(code) : nok(code);
    }
    function emailLabel(code) {
      if (code === codes.dot) {
        effects.consume(code);
        size = 0;
        return emailAtSignOrDot;
      }
      if (code === codes.greaterThan) {
        effects.exit(types.autolinkProtocol).type = types.autolinkEmail;
        effects.enter(types.autolinkMarker);
        effects.consume(code);
        effects.exit(types.autolinkMarker);
        effects.exit(types.autolink);
        return ok2;
      }
      return emailValue(code);
    }
    function emailValue(code) {
      if ((code === codes.dash || asciiAlphanumeric(code)) && size++ < constants.autolinkDomainSizeMax) {
        const next = code === codes.dash ? emailValue : emailLabel;
        effects.consume(code);
        return next;
      }
      return nok(code);
    }
  }
  // ../../node_modules/.bun/micromark-factory-space@2.1.0/node_modules/micromark-factory-space/dev/index.js
  function factorySpace(effects, ok, type, max) {
    const limit = max ? max - 1 : Infinity;
    let size = 0;
    return start;
    function start(code) {
      if (markdownSpace(code)) {
        effects.enter(type);
        return prefix(code);
      }
      return ok(code);
    }
    function prefix(code) {
      if (markdownSpace(code) && size++ < limit) {
        effects.consume(code);
        return prefix;
      }
      effects.exit(type);
      return ok(code);
    }
  }
  function factorySpaceMinMax(effects, ok, nok, type, min, max) {
    let size = 0;
    return start;
    function start(code) {
      if (max > 0 && markdownSpace(code)) {
        effects.enter(type);
        return prefix(code);
      }
      return after(code);
    }
    function prefix(code) {
      if (markdownSpace(code) && size < max) {
        effects.consume(code);
        size++;
        return prefix;
      }
      effects.exit(type);
      return after(code);
    }
    function after(code) {
      return size >= min ? ok(code) : nok(code);
    }
  }

  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/blank-line.js
  var blankLine = { partial: true, tokenize: tokenizeBlankLine };
  function tokenizeBlankLine(effects, ok, nok) {
    return start;
    function start(code) {
      return markdownSpace(code) ? factorySpace(effects, after, types.linePrefix)(code) : after(code);
    }
    function after(code) {
      return code === codes.eof || markdownLineEnding(code) ? ok(code) : nok(code);
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/block-quote.js
  var blockQuote = {
    continuation: { tokenize: tokenizeBlockQuoteContinuation },
    exit: exit2,
    name: "blockQuote",
    tokenize: tokenizeBlockQuoteStart
  };
  function tokenizeBlockQuoteStart(effects, ok2, nok) {
    const self2 = this;
    return start;
    function start(code) {
      if (code === codes.greaterThan) {
        const state = self2.containerState;
        ok(state, "expected `containerState` to be defined in container");
        if (!state.open) {
          effects.enter(types.blockQuote, { _container: true });
          state.open = true;
        }
        effects.enter(types.blockQuotePrefix);
        effects.enter(types.blockQuoteMarker);
        effects.consume(code);
        effects.exit(types.blockQuoteMarker);
        return after;
      }
      return nok(code);
    }
    function after(code) {
      if (markdownSpace(code)) {
        effects.enter(types.blockQuotePrefixWhitespace);
        effects.consume(code);
        effects.exit(types.blockQuotePrefixWhitespace);
        effects.exit(types.blockQuotePrefix);
        return ok2;
      }
      effects.exit(types.blockQuotePrefix);
      return ok2(code);
    }
  }
  function tokenizeBlockQuoteContinuation(effects, ok2, nok) {
    const self2 = this;
    return contStart;
    function contStart(code) {
      if (markdownSpace(code)) {
        ok(self2.parser.constructs.disable.null, "expected `disable.null` to be populated");
        return factorySpace(effects, contBefore, types.linePrefix, self2.parser.constructs.disable.null.includes("codeIndented") ? undefined : constants.tabSize)(code);
      }
      return contBefore(code);
    }
    function contBefore(code) {
      return effects.attempt(blockQuote, ok2, nok)(code);
    }
  }
  function exit2(effects) {
    effects.exit(types.blockQuote);
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/character-escape.js
  var characterEscape = {
    name: "characterEscape",
    tokenize: tokenizeCharacterEscape
  };
  function tokenizeCharacterEscape(effects, ok2, nok) {
    return start;
    function start(code) {
      ok(code === codes.backslash, "expected `\\`");
      effects.enter(types.characterEscape);
      effects.enter(types.escapeMarker);
      effects.consume(code);
      effects.exit(types.escapeMarker);
      return inside;
    }
    function inside(code) {
      if (asciiPunctuation(code)) {
        effects.enter(types.characterEscapeValue);
        effects.consume(code);
        effects.exit(types.characterEscapeValue);
        effects.exit(types.characterEscape);
        return ok2;
      }
      return nok(code);
    }
  }
  // ../../node_modules/.bun/decode-named-character-reference@1.3.0/node_modules/decode-named-character-reference/index.dom.js
  var element = document.createElement("i");
  function decodeNamedCharacterReference(value) {
    const characterReference = "&" + value + ";";
    element.innerHTML = characterReference;
    const character = element.textContent;
    if (character.charCodeAt(character.length - 1) === 59 && value !== "semi") {
      return false;
    }
    return character === characterReference ? false : character;
  }

  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/character-reference.js
  var characterReference = {
    name: "characterReference",
    tokenize: tokenizeCharacterReference
  };
  function tokenizeCharacterReference(effects, ok2, nok) {
    const self2 = this;
    let size = 0;
    let max;
    let test;
    return start;
    function start(code) {
      ok(code === codes.ampersand, "expected `&`");
      effects.enter(types.characterReference);
      effects.enter(types.characterReferenceMarker);
      effects.consume(code);
      effects.exit(types.characterReferenceMarker);
      return open;
    }
    function open(code) {
      if (code === codes.numberSign) {
        effects.enter(types.characterReferenceMarkerNumeric);
        effects.consume(code);
        effects.exit(types.characterReferenceMarkerNumeric);
        return numeric;
      }
      effects.enter(types.characterReferenceValue);
      max = constants.characterReferenceNamedSizeMax;
      test = asciiAlphanumeric;
      return value(code);
    }
    function numeric(code) {
      if (code === codes.uppercaseX || code === codes.lowercaseX) {
        effects.enter(types.characterReferenceMarkerHexadecimal);
        effects.consume(code);
        effects.exit(types.characterReferenceMarkerHexadecimal);
        effects.enter(types.characterReferenceValue);
        max = constants.characterReferenceHexadecimalSizeMax;
        test = asciiHexDigit;
        return value;
      }
      effects.enter(types.characterReferenceValue);
      max = constants.characterReferenceDecimalSizeMax;
      test = asciiDigit;
      return value(code);
    }
    function value(code) {
      if (code === codes.semicolon && size) {
        const token = effects.exit(types.characterReferenceValue);
        if (test === asciiAlphanumeric && !decodeNamedCharacterReference(self2.sliceSerialize(token))) {
          return nok(code);
        }
        effects.enter(types.characterReferenceMarker);
        effects.consume(code);
        effects.exit(types.characterReferenceMarker);
        effects.exit(types.characterReference);
        return ok2;
      }
      if (test(code) && size++ < max) {
        effects.consume(code);
        return value;
      }
      return nok(code);
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/partial-non-lazy-continuation.js
  var nonLazyContinuation = {
    partial: true,
    tokenize: tokenizeNonLazyContinuation
  };
  function tokenizeNonLazyContinuation(effects, ok2, nok) {
    const self2 = this;
    return start;
    function start(code) {
      if (code === codes.eof) {
        return nok(code);
      }
      ok(markdownLineEnding(code), "expected eol");
      effects.enter(types.lineEnding);
      effects.consume(code);
      effects.exit(types.lineEnding);
      return after;
    }
    function after(code) {
      return self2.parser.lazy[self2.now().line] ? nok(code) : ok2(code);
    }
  }

  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/code-fenced.js
  var codeFenced = {
    concrete: true,
    name: "codeFenced",
    tokenize: tokenizeCodeFenced
  };
  function tokenizeCodeFenced(effects, ok2, nok) {
    const self2 = this;
    const closeStart = { partial: true, tokenize: tokenizeCloseStart };
    let initialPrefix = 0;
    let sizeOpen = 0;
    let marker;
    return start;
    function start(code) {
      return beforeSequenceOpen(code);
    }
    function beforeSequenceOpen(code) {
      ok(code === codes.graveAccent || code === codes.tilde, "expected `` ` `` or `~`");
      const tail = self2.events[self2.events.length - 1];
      initialPrefix = tail && tail[1].type === types.linePrefix ? tail[2].sliceSerialize(tail[1], true).length : 0;
      marker = code;
      effects.enter(types.codeFenced);
      effects.enter(types.codeFencedFence);
      effects.enter(types.codeFencedFenceSequence);
      return sequenceOpen(code);
    }
    function sequenceOpen(code) {
      if (code === marker) {
        sizeOpen++;
        effects.consume(code);
        return sequenceOpen;
      }
      if (sizeOpen < constants.codeFencedSequenceSizeMin) {
        return nok(code);
      }
      effects.exit(types.codeFencedFenceSequence);
      return markdownSpace(code) ? factorySpace(effects, infoBefore, types.whitespace)(code) : infoBefore(code);
    }
    function infoBefore(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        effects.exit(types.codeFencedFence);
        return self2.interrupt ? ok2(code) : effects.check(nonLazyContinuation, atNonLazyBreak, after)(code);
      }
      effects.enter(types.codeFencedFenceInfo);
      effects.enter(types.chunkString, { contentType: constants.contentTypeString });
      return info(code);
    }
    function info(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        effects.exit(types.chunkString);
        effects.exit(types.codeFencedFenceInfo);
        return infoBefore(code);
      }
      if (markdownSpace(code)) {
        effects.exit(types.chunkString);
        effects.exit(types.codeFencedFenceInfo);
        return factorySpace(effects, metaBefore, types.whitespace)(code);
      }
      if (code === codes.graveAccent && code === marker) {
        return nok(code);
      }
      effects.consume(code);
      return info;
    }
    function metaBefore(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        return infoBefore(code);
      }
      effects.enter(types.codeFencedFenceMeta);
      effects.enter(types.chunkString, { contentType: constants.contentTypeString });
      return meta(code);
    }
    function meta(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        effects.exit(types.chunkString);
        effects.exit(types.codeFencedFenceMeta);
        return infoBefore(code);
      }
      if (code === codes.graveAccent && code === marker) {
        return nok(code);
      }
      effects.consume(code);
      return meta;
    }
    function atNonLazyBreak(code) {
      ok(markdownLineEnding(code), "expected eol");
      return effects.attempt(closeStart, after, contentBefore)(code);
    }
    function contentBefore(code) {
      ok(markdownLineEnding(code), "expected eol");
      effects.enter(types.lineEnding);
      effects.consume(code);
      effects.exit(types.lineEnding);
      return contentStart;
    }
    function contentStart(code) {
      return initialPrefix > 0 && markdownSpace(code) ? factorySpace(effects, beforeContentChunk, types.linePrefix, initialPrefix + 1)(code) : beforeContentChunk(code);
    }
    function beforeContentChunk(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        return effects.check(nonLazyContinuation, atNonLazyBreak, after)(code);
      }
      effects.enter(types.codeFlowValue);
      return contentChunk(code);
    }
    function contentChunk(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        effects.exit(types.codeFlowValue);
        return beforeContentChunk(code);
      }
      effects.consume(code);
      return contentChunk;
    }
    function after(code) {
      effects.exit(types.codeFenced);
      return ok2(code);
    }
    function tokenizeCloseStart(effects, ok2, nok) {
      let size = 0;
      return startBefore;
      function startBefore(code) {
        ok(markdownLineEnding(code), "expected eol");
        effects.enter(types.lineEnding);
        effects.consume(code);
        effects.exit(types.lineEnding);
        return start;
      }
      function start(code) {
        ok(self2.parser.constructs.disable.null, "expected `disable.null` to be populated");
        effects.enter(types.codeFencedFence);
        return markdownSpace(code) ? factorySpace(effects, beforeSequenceClose, types.linePrefix, self2.parser.constructs.disable.null.includes("codeIndented") ? undefined : constants.tabSize)(code) : beforeSequenceClose(code);
      }
      function beforeSequenceClose(code) {
        if (code === marker) {
          effects.enter(types.codeFencedFenceSequence);
          return sequenceClose(code);
        }
        return nok(code);
      }
      function sequenceClose(code) {
        if (code === marker) {
          size++;
          effects.consume(code);
          return sequenceClose;
        }
        if (size >= sizeOpen) {
          effects.exit(types.codeFencedFenceSequence);
          return markdownSpace(code) ? factorySpace(effects, sequenceCloseAfter, types.whitespace)(code) : sequenceCloseAfter(code);
        }
        return nok(code);
      }
      function sequenceCloseAfter(code) {
        if (code === codes.eof || markdownLineEnding(code)) {
          effects.exit(types.codeFencedFence);
          return ok2(code);
        }
        return nok(code);
      }
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/code-indented.js
  var codeIndented = {
    name: "codeIndented",
    tokenize: tokenizeCodeIndented
  };
  var furtherStart = { partial: true, tokenize: tokenizeFurtherStart };
  function tokenizeCodeIndented(effects, ok2, nok) {
    return start;
    function start(code) {
      ok(markdownSpace(code));
      effects.enter(types.codeIndented);
      return factorySpaceMinMax(effects, atBreak, nok, types.linePrefix, constants.tabSize, constants.tabSize)(code);
    }
    function atBreak(code) {
      if (code === codes.eof) {
        return after(code);
      }
      if (markdownLineEnding(code)) {
        return effects.attempt(furtherStart, atBreak, after)(code);
      }
      effects.enter(types.codeFlowValue);
      return inside(code);
    }
    function inside(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        effects.exit(types.codeFlowValue);
        return atBreak(code);
      }
      effects.consume(code);
      return inside;
    }
    function after(code) {
      effects.exit(types.codeIndented);
      return ok2(code);
    }
  }
  function tokenizeFurtherStart(effects, ok, nok) {
    const self2 = this;
    return furtherStart;
    function furtherStart(code) {
      if (self2.parser.lazy[self2.now().line]) {
        return nok(code);
      }
      if (markdownLineEnding(code)) {
        effects.enter(types.lineEnding);
        effects.consume(code);
        effects.exit(types.lineEnding);
        return furtherStart;
      }
      return factorySpaceMinMax(effects, ok, onNotEnoughPrefix, types.linePrefix, constants.tabSize, constants.tabSize)(code);
    }
    function onNotEnoughPrefix(code) {
      return markdownLineEnding(code) ? furtherStart(code) : nok(code);
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/code-text.js
  var codeText = {
    name: "codeText",
    previous: previous2,
    resolve: resolveCodeText,
    tokenize: tokenizeCodeText
  };
  function resolveCodeText(events) {
    let tailExitIndex = events.length - 4;
    let headEnterIndex = 3;
    let index;
    let enter;
    if ((events[headEnterIndex][1].type === types.lineEnding || events[headEnterIndex][1].type === "space") && (events[tailExitIndex][1].type === types.lineEnding || events[tailExitIndex][1].type === "space")) {
      index = headEnterIndex;
      while (++index < tailExitIndex) {
        if (events[index][1].type === types.codeTextData) {
          events[headEnterIndex][1].type = types.codeTextPadding;
          events[tailExitIndex][1].type = types.codeTextPadding;
          headEnterIndex += 2;
          tailExitIndex -= 2;
          break;
        }
      }
    }
    index = headEnterIndex - 1;
    tailExitIndex++;
    while (++index <= tailExitIndex) {
      if (enter === undefined) {
        if (index !== tailExitIndex && events[index][1].type !== types.lineEnding) {
          enter = index;
        }
      } else if (index === tailExitIndex || events[index][1].type === types.lineEnding) {
        events[enter][1].type = types.codeTextData;
        if (index !== enter + 2) {
          events[enter][1].end = events[index - 1][1].end;
          events.splice(enter + 2, index - enter - 2);
          tailExitIndex -= index - enter - 2;
          index = enter + 2;
        }
        enter = undefined;
      }
    }
    return events;
  }
  function previous2(code) {
    return code !== codes.graveAccent || this.events[this.events.length - 1][1].type === types.characterEscape;
  }
  function tokenizeCodeText(effects, ok2, nok) {
    const self2 = this;
    let sizeOpen = 0;
    let size;
    let token;
    return start;
    function start(code) {
      ok(code === codes.graveAccent, "expected `` ` ``");
      ok(previous2.call(self2, self2.previous), "expected correct previous");
      effects.enter(types.codeText);
      effects.enter(types.codeTextSequence);
      return sequenceOpen(code);
    }
    function sequenceOpen(code) {
      if (code === codes.graveAccent) {
        effects.consume(code);
        sizeOpen++;
        return sequenceOpen;
      }
      effects.exit(types.codeTextSequence);
      return between(code);
    }
    function between(code) {
      if (code === codes.eof) {
        return nok(code);
      }
      if (code === codes.space) {
        effects.enter("space");
        effects.consume(code);
        effects.exit("space");
        return between;
      }
      if (code === codes.graveAccent) {
        token = effects.enter(types.codeTextSequence);
        size = 0;
        return sequenceClose(code);
      }
      if (markdownLineEnding(code)) {
        effects.enter(types.lineEnding);
        effects.consume(code);
        effects.exit(types.lineEnding);
        return between;
      }
      effects.enter(types.codeTextData);
      return data(code);
    }
    function data(code) {
      if (code === codes.eof || code === codes.space || code === codes.graveAccent || markdownLineEnding(code)) {
        effects.exit(types.codeTextData);
        return between(code);
      }
      effects.consume(code);
      return data;
    }
    function sequenceClose(code) {
      if (code === codes.graveAccent) {
        effects.consume(code);
        size++;
        return sequenceClose;
      }
      if (size === sizeOpen) {
        effects.exit(types.codeTextSequence);
        effects.exit(types.codeText);
        return ok2(code);
      }
      token.type = types.codeTextData;
      return data(code);
    }
  }
  // ../../node_modules/.bun/micromark-util-subtokenize@2.1.0/node_modules/micromark-util-subtokenize/dev/lib/splice-buffer.js
  class SpliceBuffer {
    constructor(initial) {
      this.left = initial ? [...initial] : [];
      this.right = [];
    }
    get(index) {
      if (index < 0 || index >= this.left.length + this.right.length) {
        throw new RangeError("Cannot access index `" + index + "` in a splice buffer of size `" + (this.left.length + this.right.length) + "`");
      }
      if (index < this.left.length)
        return this.left[index];
      return this.right[this.right.length - index + this.left.length - 1];
    }
    get length() {
      return this.left.length + this.right.length;
    }
    shift() {
      this.setCursor(0);
      return this.right.pop();
    }
    slice(start, end) {
      const stop = end === null || end === undefined ? Number.POSITIVE_INFINITY : end;
      if (stop < this.left.length) {
        return this.left.slice(start, stop);
      }
      if (start > this.left.length) {
        return this.right.slice(this.right.length - stop + this.left.length, this.right.length - start + this.left.length).reverse();
      }
      return this.left.slice(start).concat(this.right.slice(this.right.length - stop + this.left.length).reverse());
    }
    splice(start, deleteCount, items) {
      const count = deleteCount || 0;
      this.setCursor(Math.trunc(start));
      const removed = this.right.splice(this.right.length - count, Number.POSITIVE_INFINITY);
      if (items)
        chunkedPush(this.left, items);
      return removed.reverse();
    }
    pop() {
      this.setCursor(Number.POSITIVE_INFINITY);
      return this.left.pop();
    }
    push(item) {
      this.setCursor(Number.POSITIVE_INFINITY);
      this.left.push(item);
    }
    pushMany(items) {
      this.setCursor(Number.POSITIVE_INFINITY);
      chunkedPush(this.left, items);
    }
    unshift(item) {
      this.setCursor(0);
      this.right.push(item);
    }
    unshiftMany(items) {
      this.setCursor(0);
      chunkedPush(this.right, items.reverse());
    }
    setCursor(n) {
      if (n === this.left.length || n > this.left.length && this.right.length === 0 || n < 0 && this.left.length === 0)
        return;
      if (n < this.left.length) {
        const removed = this.left.splice(n, Number.POSITIVE_INFINITY);
        chunkedPush(this.right, removed.reverse());
      } else {
        const removed = this.right.splice(this.left.length + this.right.length - n, Number.POSITIVE_INFINITY);
        chunkedPush(this.left, removed.reverse());
      }
    }
  }
  function chunkedPush(list, right) {
    let chunkStart = 0;
    if (right.length < constants.v8MaxSafeChunkSize) {
      list.push(...right);
    } else {
      while (chunkStart < right.length) {
        list.push(...right.slice(chunkStart, chunkStart + constants.v8MaxSafeChunkSize));
        chunkStart += constants.v8MaxSafeChunkSize;
      }
    }
  }

  // ../../node_modules/.bun/micromark-util-subtokenize@2.1.0/node_modules/micromark-util-subtokenize/dev/index.js
  function subtokenize(eventsArray) {
    const jumps = {};
    let index = -1;
    let event;
    let lineIndex;
    let otherIndex;
    let otherEvent;
    let parameters;
    let subevents;
    let more;
    const events = new SpliceBuffer(eventsArray);
    while (++index < events.length) {
      while (index in jumps) {
        index = jumps[index];
      }
      event = events.get(index);
      if (index && event[1].type === types.chunkFlow && events.get(index - 1)[1].type === types.listItemPrefix) {
        ok(event[1]._tokenizer, "expected `_tokenizer` on subtokens");
        subevents = event[1]._tokenizer.events;
        otherIndex = 0;
        if (otherIndex < subevents.length && subevents[otherIndex][1].type === types.lineEndingBlank) {
          otherIndex += 2;
        }
        if (otherIndex < subevents.length && subevents[otherIndex][1].type === types.content) {
          while (++otherIndex < subevents.length) {
            if (subevents[otherIndex][1].type === types.content) {
              break;
            }
            if (subevents[otherIndex][1].type === types.chunkText) {
              subevents[otherIndex][1]._isInFirstContentOfListItem = true;
              otherIndex++;
            }
          }
        }
      }
      if (event[0] === "enter") {
        if (event[1].contentType) {
          Object.assign(jumps, subcontent(events, index));
          index = jumps[index];
          more = true;
        }
      } else if (event[1]._container) {
        otherIndex = index;
        lineIndex = undefined;
        while (otherIndex--) {
          otherEvent = events.get(otherIndex);
          if (otherEvent[1].type === types.lineEnding || otherEvent[1].type === types.lineEndingBlank) {
            if (otherEvent[0] === "enter") {
              if (lineIndex) {
                events.get(lineIndex)[1].type = types.lineEndingBlank;
              }
              otherEvent[1].type = types.lineEnding;
              lineIndex = otherIndex;
            }
          } else if (otherEvent[1].type === types.linePrefix || otherEvent[1].type === types.listItemIndent) {} else {
            break;
          }
        }
        if (lineIndex) {
          event[1].end = { ...events.get(lineIndex)[1].start };
          parameters = events.slice(lineIndex, index);
          parameters.unshift(event);
          events.splice(lineIndex, index - lineIndex + 1, parameters);
        }
      }
    }
    splice(eventsArray, 0, Number.POSITIVE_INFINITY, events.slice(0));
    return !more;
  }
  function subcontent(events, eventIndex) {
    const token = events.get(eventIndex)[1];
    const context = events.get(eventIndex)[2];
    let startPosition = eventIndex - 1;
    const startPositions = [];
    ok(token.contentType, "expected `contentType` on subtokens");
    let tokenizer = token._tokenizer;
    if (!tokenizer) {
      tokenizer = context.parser[token.contentType](token.start);
      if (token._contentTypeTextTrailing) {
        tokenizer._contentTypeTextTrailing = true;
      }
    }
    const childEvents = tokenizer.events;
    const jumps = [];
    const gaps = {};
    let stream;
    let previous;
    let index = -1;
    let current = token;
    let adjust = 0;
    let start = 0;
    const breaks = [start];
    while (current) {
      while (events.get(++startPosition)[1] !== current) {}
      ok(!previous || current.previous === previous, "expected previous to match");
      ok(!previous || previous.next === current, "expected next to match");
      startPositions.push(startPosition);
      if (!current._tokenizer) {
        stream = context.sliceStream(current);
        if (!current.next) {
          stream.push(codes.eof);
        }
        if (previous) {
          tokenizer.defineSkip(current.start);
        }
        if (current._isInFirstContentOfListItem) {
          tokenizer._gfmTasklistFirstContentOfListItem = true;
        }
        tokenizer.write(stream);
        if (current._isInFirstContentOfListItem) {
          tokenizer._gfmTasklistFirstContentOfListItem = undefined;
        }
      }
      previous = current;
      current = current.next;
    }
    current = token;
    while (++index < childEvents.length) {
      if (childEvents[index][0] === "exit" && childEvents[index - 1][0] === "enter" && childEvents[index][1].type === childEvents[index - 1][1].type && childEvents[index][1].start.line !== childEvents[index][1].end.line) {
        ok(current, "expected a current token");
        start = index + 1;
        breaks.push(start);
        current._tokenizer = undefined;
        current.previous = undefined;
        current = current.next;
      }
    }
    tokenizer.events = [];
    if (current) {
      current._tokenizer = undefined;
      current.previous = undefined;
      ok(!current.next, "expected no next token");
    } else {
      breaks.pop();
    }
    index = breaks.length;
    while (index--) {
      const slice = childEvents.slice(breaks[index], breaks[index + 1]);
      const start = startPositions.pop();
      ok(start !== undefined, "expected a start position when splicing");
      jumps.push([start, start + slice.length - 1]);
      events.splice(start, 2, slice);
    }
    jumps.reverse();
    index = -1;
    while (++index < jumps.length) {
      gaps[adjust + jumps[index][0]] = adjust + jumps[index][1];
      adjust += jumps[index][1] - jumps[index][0] - 1;
    }
    return gaps;
  }

  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/content.js
  var content = { resolve: resolveContent, tokenize: tokenizeContent };
  var continuationConstruct = { partial: true, tokenize: tokenizeContinuation };
  function resolveContent(events) {
    subtokenize(events);
    return events;
  }
  function tokenizeContent(effects, ok2) {
    let previous;
    return chunkStart;
    function chunkStart(code) {
      ok(code !== codes.eof && !markdownLineEnding(code), "expected no eof or eol");
      effects.enter(types.content);
      previous = effects.enter(types.chunkContent, {
        contentType: constants.contentTypeContent
      });
      return chunkInside(code);
    }
    function chunkInside(code) {
      if (code === codes.eof) {
        return contentEnd(code);
      }
      if (markdownLineEnding(code)) {
        return effects.check(continuationConstruct, contentContinue, contentEnd)(code);
      }
      effects.consume(code);
      return chunkInside;
    }
    function contentEnd(code) {
      effects.exit(types.chunkContent);
      effects.exit(types.content);
      return ok2(code);
    }
    function contentContinue(code) {
      ok(markdownLineEnding(code), "expected eol");
      effects.consume(code);
      effects.exit(types.chunkContent);
      ok(previous, "expected previous token");
      previous.next = effects.enter(types.chunkContent, {
        contentType: constants.contentTypeContent,
        previous
      });
      previous = previous.next;
      return chunkInside;
    }
  }
  function tokenizeContinuation(effects, ok2, nok) {
    const self2 = this;
    return startLookahead;
    function startLookahead(code) {
      ok(markdownLineEnding(code), "expected a line ending");
      effects.exit(types.chunkContent);
      effects.enter(types.lineEnding);
      effects.consume(code);
      effects.exit(types.lineEnding);
      return factorySpace(effects, prefixed, types.linePrefix);
    }
    function prefixed(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        return nok(code);
      }
      ok(self2.parser.constructs.disable.null, "expected `disable.null` to be populated");
      const tail = self2.events[self2.events.length - 1];
      if (!self2.parser.constructs.disable.null.includes("codeIndented") && tail && tail[1].type === types.linePrefix && tail[2].sliceSerialize(tail[1], true).length >= constants.tabSize) {
        return ok2(code);
      }
      return effects.interrupt(self2.parser.constructs.flow, nok, ok2)(code);
    }
  }
  // ../../node_modules/.bun/micromark-factory-destination@2.0.1/node_modules/micromark-factory-destination/dev/index.js
  function factoryDestination(effects, ok, nok, type, literalType, literalMarkerType, rawType, stringType, max) {
    const limit = max || Number.POSITIVE_INFINITY;
    let balance = 0;
    return start;
    function start(code) {
      if (code === codes.lessThan) {
        effects.enter(type);
        effects.enter(literalType);
        effects.enter(literalMarkerType);
        effects.consume(code);
        effects.exit(literalMarkerType);
        return enclosedBefore;
      }
      if (code === codes.eof || code === codes.space || code === codes.rightParenthesis || asciiControl(code)) {
        return nok(code);
      }
      effects.enter(type);
      effects.enter(rawType);
      effects.enter(stringType);
      effects.enter(types.chunkString, { contentType: constants.contentTypeString });
      return raw(code);
    }
    function enclosedBefore(code) {
      if (code === codes.greaterThan) {
        effects.enter(literalMarkerType);
        effects.consume(code);
        effects.exit(literalMarkerType);
        effects.exit(literalType);
        effects.exit(type);
        return ok;
      }
      effects.enter(stringType);
      effects.enter(types.chunkString, { contentType: constants.contentTypeString });
      return enclosed(code);
    }
    function enclosed(code) {
      if (code === codes.greaterThan) {
        effects.exit(types.chunkString);
        effects.exit(stringType);
        return enclosedBefore(code);
      }
      if (code === codes.eof || code === codes.lessThan || markdownLineEnding(code)) {
        return nok(code);
      }
      effects.consume(code);
      return code === codes.backslash ? enclosedEscape : enclosed;
    }
    function enclosedEscape(code) {
      if (code === codes.lessThan || code === codes.greaterThan || code === codes.backslash) {
        effects.consume(code);
        return enclosed;
      }
      return enclosed(code);
    }
    function raw(code) {
      if (!balance && (code === codes.eof || code === codes.rightParenthesis || markdownLineEndingOrSpace(code))) {
        effects.exit(types.chunkString);
        effects.exit(stringType);
        effects.exit(rawType);
        effects.exit(type);
        return ok(code);
      }
      if (balance < limit && code === codes.leftParenthesis) {
        effects.consume(code);
        balance++;
        return raw;
      }
      if (code === codes.rightParenthesis) {
        effects.consume(code);
        balance--;
        return raw;
      }
      if (code === codes.eof || code === codes.space || code === codes.leftParenthesis || asciiControl(code)) {
        return nok(code);
      }
      effects.consume(code);
      return code === codes.backslash ? rawEscape : raw;
    }
    function rawEscape(code) {
      if (code === codes.leftParenthesis || code === codes.rightParenthesis || code === codes.backslash) {
        effects.consume(code);
        return raw;
      }
      return raw(code);
    }
  }

  // ../../node_modules/.bun/micromark-factory-label@2.0.1/node_modules/micromark-factory-label/dev/index.js
  function factoryLabel(effects, ok2, nok, type, markerType, stringType) {
    const self2 = this;
    let size = 0;
    let seen;
    return start;
    function start(code) {
      ok(code === codes.leftSquareBracket, "expected `[`");
      effects.enter(type);
      effects.enter(markerType);
      effects.consume(code);
      effects.exit(markerType);
      effects.enter(stringType);
      return atBreak;
    }
    function atBreak(code) {
      if (size > constants.linkReferenceSizeMax || code === codes.eof || code === codes.leftSquareBracket || code === codes.rightSquareBracket && !seen || code === codes.caret && !size && "_hiddenFootnoteSupport" in self2.parser.constructs) {
        return nok(code);
      }
      if (code === codes.rightSquareBracket) {
        effects.exit(stringType);
        effects.enter(markerType);
        effects.consume(code);
        effects.exit(markerType);
        effects.exit(type);
        return ok2;
      }
      if (markdownLineEnding(code)) {
        effects.enter(types.lineEnding);
        effects.consume(code);
        effects.exit(types.lineEnding);
        return atBreak;
      }
      effects.enter(types.chunkString, { contentType: constants.contentTypeString });
      return labelInside(code);
    }
    function labelInside(code) {
      if (code === codes.eof || code === codes.leftSquareBracket || code === codes.rightSquareBracket || markdownLineEnding(code) || size++ > constants.linkReferenceSizeMax) {
        effects.exit(types.chunkString);
        return atBreak(code);
      }
      effects.consume(code);
      if (!seen)
        seen = !markdownSpace(code);
      return code === codes.backslash ? labelEscape : labelInside;
    }
    function labelEscape(code) {
      if (code === codes.leftSquareBracket || code === codes.backslash || code === codes.rightSquareBracket) {
        effects.consume(code);
        size++;
        return labelInside;
      }
      return labelInside(code);
    }
  }

  // ../../node_modules/.bun/micromark-factory-title@2.0.1/node_modules/micromark-factory-title/dev/index.js
  function factoryTitle(effects, ok, nok, type, markerType, stringType) {
    let marker;
    return start;
    function start(code) {
      if (code === codes.quotationMark || code === codes.apostrophe || code === codes.leftParenthesis) {
        effects.enter(type);
        effects.enter(markerType);
        effects.consume(code);
        effects.exit(markerType);
        marker = code === codes.leftParenthesis ? codes.rightParenthesis : code;
        return begin;
      }
      return nok(code);
    }
    function begin(code) {
      if (code === marker) {
        effects.enter(markerType);
        effects.consume(code);
        effects.exit(markerType);
        effects.exit(type);
        return ok;
      }
      effects.enter(stringType);
      return atBreak(code);
    }
    function atBreak(code) {
      if (code === marker) {
        effects.exit(stringType);
        return begin(marker);
      }
      if (code === codes.eof) {
        return nok(code);
      }
      if (markdownLineEnding(code)) {
        effects.enter(types.lineEnding);
        effects.consume(code);
        effects.exit(types.lineEnding);
        return factorySpace(effects, atBreak, types.linePrefix);
      }
      effects.enter(types.chunkString, { contentType: constants.contentTypeString });
      return inside(code);
    }
    function inside(code) {
      if (code === marker || code === codes.eof || markdownLineEnding(code)) {
        effects.exit(types.chunkString);
        return atBreak(code);
      }
      effects.consume(code);
      return code === codes.backslash ? escape : inside;
    }
    function escape(code) {
      if (code === marker || code === codes.backslash) {
        effects.consume(code);
        return inside;
      }
      return inside(code);
    }
  }

  // ../../node_modules/.bun/micromark-factory-whitespace@2.0.1/node_modules/micromark-factory-whitespace/dev/index.js
  function factoryWhitespace(effects, ok) {
    let seen;
    return start;
    function start(code) {
      if (markdownLineEnding(code)) {
        effects.enter(types.lineEnding);
        effects.consume(code);
        effects.exit(types.lineEnding);
        seen = true;
        return start;
      }
      if (markdownSpace(code)) {
        return factorySpace(effects, start, seen ? types.linePrefix : types.lineSuffix)(code);
      }
      return ok(code);
    }
  }

  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/definition.js
  var definition2 = { name: "definition", tokenize: tokenizeDefinition };
  var titleBefore = { partial: true, tokenize: tokenizeTitleBefore };
  function tokenizeDefinition(effects, ok2, nok) {
    const self2 = this;
    let identifier;
    return start;
    function start(code) {
      effects.enter(types.definition);
      return before(code);
    }
    function before(code) {
      ok(code === codes.leftSquareBracket, "expected `[`");
      return factoryLabel.call(self2, effects, labelAfter, nok, types.definitionLabel, types.definitionLabelMarker, types.definitionLabelString)(code);
    }
    function labelAfter(code) {
      identifier = normalizeIdentifier(self2.sliceSerialize(self2.events[self2.events.length - 1][1]).slice(1, -1));
      if (code === codes.colon) {
        effects.enter(types.definitionMarker);
        effects.consume(code);
        effects.exit(types.definitionMarker);
        return markerAfter;
      }
      return nok(code);
    }
    function markerAfter(code) {
      return markdownLineEndingOrSpace(code) ? factoryWhitespace(effects, destinationBefore)(code) : destinationBefore(code);
    }
    function destinationBefore(code) {
      return factoryDestination(effects, destinationAfter, nok, types.definitionDestination, types.definitionDestinationLiteral, types.definitionDestinationLiteralMarker, types.definitionDestinationRaw, types.definitionDestinationString)(code);
    }
    function destinationAfter(code) {
      return effects.attempt(titleBefore, after, after)(code);
    }
    function after(code) {
      return markdownSpace(code) ? factorySpace(effects, afterWhitespace, types.whitespace)(code) : afterWhitespace(code);
    }
    function afterWhitespace(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        effects.exit(types.definition);
        self2.parser.defined.push(identifier);
        return ok2(code);
      }
      return nok(code);
    }
  }
  function tokenizeTitleBefore(effects, ok, nok) {
    return titleBefore;
    function titleBefore(code) {
      return markdownLineEndingOrSpace(code) ? factoryWhitespace(effects, beforeMarker)(code) : nok(code);
    }
    function beforeMarker(code) {
      return factoryTitle(effects, titleAfter, nok, types.definitionTitle, types.definitionTitleMarker, types.definitionTitleString)(code);
    }
    function titleAfter(code) {
      return markdownSpace(code) ? factorySpace(effects, titleAfterOptionalWhitespace, types.whitespace)(code) : titleAfterOptionalWhitespace(code);
    }
    function titleAfterOptionalWhitespace(code) {
      return code === codes.eof || markdownLineEnding(code) ? ok(code) : nok(code);
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/hard-break-escape.js
  var hardBreakEscape = {
    name: "hardBreakEscape",
    tokenize: tokenizeHardBreakEscape
  };
  function tokenizeHardBreakEscape(effects, ok2, nok) {
    return start;
    function start(code) {
      ok(code === codes.backslash, "expected `\\`");
      effects.enter(types.hardBreakEscape);
      effects.consume(code);
      return after;
    }
    function after(code) {
      if (markdownLineEnding(code)) {
        effects.exit(types.hardBreakEscape);
        return ok2(code);
      }
      return nok(code);
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/heading-atx.js
  var headingAtx = {
    name: "headingAtx",
    resolve: resolveHeadingAtx,
    tokenize: tokenizeHeadingAtx
  };
  function resolveHeadingAtx(events, context) {
    let contentEnd = events.length - 2;
    let contentStart = 3;
    if (events[contentStart][1].type === types.whitespace) {
      contentStart += 2;
    }
    if (contentEnd - 2 > contentStart && events[contentEnd][1].type === types.whitespace) {
      contentEnd -= 2;
    }
    if (events[contentEnd][1].type === types.atxHeadingSequence && (contentStart === contentEnd - 1 || contentEnd - 4 > contentStart && events[contentEnd - 2][1].type === types.whitespace)) {
      contentEnd -= contentStart + 1 === contentEnd ? 2 : 4;
    }
    if (contentEnd > contentStart) {
      const content = {
        type: types.atxHeadingText,
        start: events[contentStart][1].start,
        end: events[contentEnd][1].end
      };
      const text = {
        type: types.chunkText,
        start: events[contentStart][1].start,
        end: events[contentEnd][1].end,
        contentType: constants.contentTypeText
      };
      splice(events, contentStart, contentEnd - contentStart + 1, [
        ["enter", content, context],
        ["enter", text, context],
        ["exit", text, context],
        ["exit", content, context]
      ]);
    }
    return events;
  }
  function tokenizeHeadingAtx(effects, ok2, nok) {
    let size = 0;
    return start;
    function start(code) {
      effects.enter(types.atxHeading);
      return before(code);
    }
    function before(code) {
      ok(code === codes.numberSign, "expected `#`");
      effects.enter(types.atxHeadingSequence);
      return sequenceOpen(code);
    }
    function sequenceOpen(code) {
      if (code === codes.numberSign && size++ < constants.atxHeadingOpeningFenceSizeMax) {
        effects.consume(code);
        return sequenceOpen;
      }
      if (code === codes.eof || markdownLineEndingOrSpace(code)) {
        effects.exit(types.atxHeadingSequence);
        return atBreak(code);
      }
      return nok(code);
    }
    function atBreak(code) {
      if (code === codes.numberSign) {
        effects.enter(types.atxHeadingSequence);
        return sequenceFurther(code);
      }
      if (code === codes.eof || markdownLineEnding(code)) {
        effects.exit(types.atxHeading);
        return ok2(code);
      }
      if (markdownSpace(code)) {
        return factorySpace(effects, atBreak, types.whitespace)(code);
      }
      effects.enter(types.atxHeadingText);
      return data(code);
    }
    function sequenceFurther(code) {
      if (code === codes.numberSign) {
        effects.consume(code);
        return sequenceFurther;
      }
      effects.exit(types.atxHeadingSequence);
      return atBreak(code);
    }
    function data(code) {
      if (code === codes.eof || code === codes.numberSign || markdownLineEndingOrSpace(code)) {
        effects.exit(types.atxHeadingText);
        return atBreak(code);
      }
      effects.consume(code);
      return data;
    }
  }
  // ../../node_modules/.bun/micromark-util-html-tag-name@2.0.1/node_modules/micromark-util-html-tag-name/index.js
  var htmlBlockNames = [
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul"
  ];
  var htmlRawNames = ["pre", "script", "style", "textarea"];

  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/html-flow.js
  var htmlFlow = {
    concrete: true,
    name: "htmlFlow",
    resolveTo: resolveToHtmlFlow,
    tokenize: tokenizeHtmlFlow
  };
  var blankLineBefore = { partial: true, tokenize: tokenizeBlankLineBefore };
  function resolveToHtmlFlow(events) {
    let index = events.length;
    while (index--) {
      if (events[index][0] === "enter" && events[index][1].type === types.htmlFlow) {
        break;
      }
    }
    if (index > 1 && events[index - 2][1].type === types.linePrefix) {
      events[index][1].start = events[index - 2][1].start;
      events[index + 1][1].start = events[index - 2][1].start;
      events.splice(index - 2, 2);
    }
    return events;
  }
  function tokenizeHtmlFlow(effects, ok2, nok) {
    const self2 = this;
    let marker;
    let closingTag;
    let buffer;
    let index;
    let markerB;
    return start;
    function start(code) {
      return before(code);
    }
    function before(code) {
      ok(code === codes.lessThan, "expected `<`");
      effects.enter(types.htmlFlow);
      effects.enter(types.htmlFlowData);
      effects.consume(code);
      return open;
    }
    function open(code) {
      if (code === codes.exclamationMark) {
        effects.consume(code);
        return declarationOpen;
      }
      if (code === codes.slash) {
        effects.consume(code);
        closingTag = true;
        return tagCloseStart;
      }
      if (code === codes.questionMark) {
        effects.consume(code);
        marker = constants.htmlInstruction;
        return self2.interrupt ? ok2 : continuationDeclarationInside;
      }
      if (asciiAlpha(code)) {
        ok(code !== null);
        effects.consume(code);
        buffer = String.fromCharCode(code);
        return tagName;
      }
      return nok(code);
    }
    function declarationOpen(code) {
      if (code === codes.dash) {
        effects.consume(code);
        marker = constants.htmlComment;
        return commentOpenInside;
      }
      if (code === codes.leftSquareBracket) {
        effects.consume(code);
        marker = constants.htmlCdata;
        index = 0;
        return cdataOpenInside;
      }
      if (asciiAlpha(code)) {
        effects.consume(code);
        marker = constants.htmlDeclaration;
        return self2.interrupt ? ok2 : continuationDeclarationInside;
      }
      return nok(code);
    }
    function commentOpenInside(code) {
      if (code === codes.dash) {
        effects.consume(code);
        return self2.interrupt ? ok2 : continuationDeclarationInside;
      }
      return nok(code);
    }
    function cdataOpenInside(code) {
      const value = constants.cdataOpeningString;
      if (code === value.charCodeAt(index++)) {
        effects.consume(code);
        if (index === value.length) {
          return self2.interrupt ? ok2 : continuation;
        }
        return cdataOpenInside;
      }
      return nok(code);
    }
    function tagCloseStart(code) {
      if (asciiAlpha(code)) {
        ok(code !== null);
        effects.consume(code);
        buffer = String.fromCharCode(code);
        return tagName;
      }
      return nok(code);
    }
    function tagName(code) {
      if (code === codes.eof || code === codes.slash || code === codes.greaterThan || markdownLineEndingOrSpace(code)) {
        const slash = code === codes.slash;
        const name = buffer.toLowerCase();
        if (!slash && !closingTag && htmlRawNames.includes(name)) {
          marker = constants.htmlRaw;
          return self2.interrupt ? ok2(code) : continuation(code);
        }
        if (htmlBlockNames.includes(buffer.toLowerCase())) {
          marker = constants.htmlBasic;
          if (slash) {
            effects.consume(code);
            return basicSelfClosing;
          }
          return self2.interrupt ? ok2(code) : continuation(code);
        }
        marker = constants.htmlComplete;
        return self2.interrupt && !self2.parser.lazy[self2.now().line] ? nok(code) : closingTag ? completeClosingTagAfter(code) : completeAttributeNameBefore(code);
      }
      if (code === codes.dash || asciiAlphanumeric(code)) {
        effects.consume(code);
        buffer += String.fromCharCode(code);
        return tagName;
      }
      return nok(code);
    }
    function basicSelfClosing(code) {
      if (code === codes.greaterThan) {
        effects.consume(code);
        return self2.interrupt ? ok2 : continuation;
      }
      return nok(code);
    }
    function completeClosingTagAfter(code) {
      if (markdownSpace(code)) {
        effects.consume(code);
        return completeClosingTagAfter;
      }
      return completeEnd(code);
    }
    function completeAttributeNameBefore(code) {
      if (code === codes.slash) {
        effects.consume(code);
        return completeEnd;
      }
      if (code === codes.colon || code === codes.underscore || asciiAlpha(code)) {
        effects.consume(code);
        return completeAttributeName;
      }
      if (markdownSpace(code)) {
        effects.consume(code);
        return completeAttributeNameBefore;
      }
      return completeEnd(code);
    }
    function completeAttributeName(code) {
      if (code === codes.dash || code === codes.dot || code === codes.colon || code === codes.underscore || asciiAlphanumeric(code)) {
        effects.consume(code);
        return completeAttributeName;
      }
      return completeAttributeNameAfter(code);
    }
    function completeAttributeNameAfter(code) {
      if (code === codes.equalsTo) {
        effects.consume(code);
        return completeAttributeValueBefore;
      }
      if (markdownSpace(code)) {
        effects.consume(code);
        return completeAttributeNameAfter;
      }
      return completeAttributeNameBefore(code);
    }
    function completeAttributeValueBefore(code) {
      if (code === codes.eof || code === codes.lessThan || code === codes.equalsTo || code === codes.greaterThan || code === codes.graveAccent) {
        return nok(code);
      }
      if (code === codes.quotationMark || code === codes.apostrophe) {
        effects.consume(code);
        markerB = code;
        return completeAttributeValueQuoted;
      }
      if (markdownSpace(code)) {
        effects.consume(code);
        return completeAttributeValueBefore;
      }
      return completeAttributeValueUnquoted(code);
    }
    function completeAttributeValueQuoted(code) {
      if (code === markerB) {
        effects.consume(code);
        markerB = null;
        return completeAttributeValueQuotedAfter;
      }
      if (code === codes.eof || markdownLineEnding(code)) {
        return nok(code);
      }
      effects.consume(code);
      return completeAttributeValueQuoted;
    }
    function completeAttributeValueUnquoted(code) {
      if (code === codes.eof || code === codes.quotationMark || code === codes.apostrophe || code === codes.slash || code === codes.lessThan || code === codes.equalsTo || code === codes.greaterThan || code === codes.graveAccent || markdownLineEndingOrSpace(code)) {
        return completeAttributeNameAfter(code);
      }
      effects.consume(code);
      return completeAttributeValueUnquoted;
    }
    function completeAttributeValueQuotedAfter(code) {
      if (code === codes.slash || code === codes.greaterThan || markdownSpace(code)) {
        return completeAttributeNameBefore(code);
      }
      return nok(code);
    }
    function completeEnd(code) {
      if (code === codes.greaterThan) {
        effects.consume(code);
        return completeAfter;
      }
      return nok(code);
    }
    function completeAfter(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        return continuation(code);
      }
      if (markdownSpace(code)) {
        effects.consume(code);
        return completeAfter;
      }
      return nok(code);
    }
    function continuation(code) {
      if (code === codes.dash && marker === constants.htmlComment) {
        effects.consume(code);
        return continuationCommentInside;
      }
      if (code === codes.lessThan && marker === constants.htmlRaw) {
        effects.consume(code);
        return continuationRawTagOpen;
      }
      if (code === codes.greaterThan && marker === constants.htmlDeclaration) {
        effects.consume(code);
        return continuationClose;
      }
      if (code === codes.questionMark && marker === constants.htmlInstruction) {
        effects.consume(code);
        return continuationDeclarationInside;
      }
      if (code === codes.rightSquareBracket && marker === constants.htmlCdata) {
        effects.consume(code);
        return continuationCdataInside;
      }
      if (markdownLineEnding(code) && (marker === constants.htmlBasic || marker === constants.htmlComplete)) {
        effects.exit(types.htmlFlowData);
        return effects.check(blankLineBefore, continuationAfter, continuationStart)(code);
      }
      if (code === codes.eof || markdownLineEnding(code)) {
        effects.exit(types.htmlFlowData);
        return continuationStart(code);
      }
      effects.consume(code);
      return continuation;
    }
    function continuationStart(code) {
      return effects.check(nonLazyContinuation, continuationStartNonLazy, continuationAfter)(code);
    }
    function continuationStartNonLazy(code) {
      ok(markdownLineEnding(code));
      effects.enter(types.lineEnding);
      effects.consume(code);
      effects.exit(types.lineEnding);
      return continuationBefore;
    }
    function continuationBefore(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        return continuationStart(code);
      }
      effects.enter(types.htmlFlowData);
      return continuation(code);
    }
    function continuationCommentInside(code) {
      if (code === codes.dash) {
        effects.consume(code);
        return continuationDeclarationInside;
      }
      return continuation(code);
    }
    function continuationRawTagOpen(code) {
      if (code === codes.slash) {
        effects.consume(code);
        buffer = "";
        return continuationRawEndTag;
      }
      return continuation(code);
    }
    function continuationRawEndTag(code) {
      if (code === codes.greaterThan) {
        const name = buffer.toLowerCase();
        if (htmlRawNames.includes(name)) {
          effects.consume(code);
          return continuationClose;
        }
        return continuation(code);
      }
      if (asciiAlpha(code) && buffer.length < constants.htmlRawSizeMax) {
        ok(code !== null);
        effects.consume(code);
        buffer += String.fromCharCode(code);
        return continuationRawEndTag;
      }
      return continuation(code);
    }
    function continuationCdataInside(code) {
      if (code === codes.rightSquareBracket) {
        effects.consume(code);
        return continuationDeclarationInside;
      }
      return continuation(code);
    }
    function continuationDeclarationInside(code) {
      if (code === codes.greaterThan) {
        effects.consume(code);
        return continuationClose;
      }
      if (code === codes.dash && marker === constants.htmlComment) {
        effects.consume(code);
        return continuationDeclarationInside;
      }
      return continuation(code);
    }
    function continuationClose(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        effects.exit(types.htmlFlowData);
        return continuationAfter(code);
      }
      effects.consume(code);
      return continuationClose;
    }
    function continuationAfter(code) {
      effects.exit(types.htmlFlow);
      return ok2(code);
    }
  }
  function tokenizeBlankLineBefore(effects, ok2, nok) {
    return start;
    function start(code) {
      ok(markdownLineEnding(code), "expected a line ending");
      effects.enter(types.lineEnding);
      effects.consume(code);
      effects.exit(types.lineEnding);
      return effects.attempt(blankLine, ok2, nok);
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/html-text.js
  var htmlText = { name: "htmlText", tokenize: tokenizeHtmlText };
  function tokenizeHtmlText(effects, ok2, nok) {
    const self2 = this;
    let marker;
    let index;
    let returnState;
    return start;
    function start(code) {
      ok(code === codes.lessThan, "expected `<`");
      effects.enter(types.htmlText);
      effects.enter(types.htmlTextData);
      effects.consume(code);
      return open;
    }
    function open(code) {
      if (code === codes.exclamationMark) {
        effects.consume(code);
        return declarationOpen;
      }
      if (code === codes.slash) {
        effects.consume(code);
        return tagCloseStart;
      }
      if (code === codes.questionMark) {
        effects.consume(code);
        return instruction;
      }
      if (asciiAlpha(code)) {
        effects.consume(code);
        return tagOpen;
      }
      return nok(code);
    }
    function declarationOpen(code) {
      if (code === codes.dash) {
        effects.consume(code);
        return commentOpenInside;
      }
      if (code === codes.leftSquareBracket) {
        effects.consume(code);
        index = 0;
        return cdataOpenInside;
      }
      if (asciiAlpha(code)) {
        effects.consume(code);
        return declaration;
      }
      return nok(code);
    }
    function commentOpenInside(code) {
      if (code === codes.dash) {
        effects.consume(code);
        return commentEnd;
      }
      return nok(code);
    }
    function comment(code) {
      if (code === codes.eof) {
        return nok(code);
      }
      if (code === codes.dash) {
        effects.consume(code);
        return commentClose;
      }
      if (markdownLineEnding(code)) {
        returnState = comment;
        return lineEndingBefore(code);
      }
      effects.consume(code);
      return comment;
    }
    function commentClose(code) {
      if (code === codes.dash) {
        effects.consume(code);
        return commentEnd;
      }
      return comment(code);
    }
    function commentEnd(code) {
      return code === codes.greaterThan ? end(code) : code === codes.dash ? commentClose(code) : comment(code);
    }
    function cdataOpenInside(code) {
      const value = constants.cdataOpeningString;
      if (code === value.charCodeAt(index++)) {
        effects.consume(code);
        return index === value.length ? cdata : cdataOpenInside;
      }
      return nok(code);
    }
    function cdata(code) {
      if (code === codes.eof) {
        return nok(code);
      }
      if (code === codes.rightSquareBracket) {
        effects.consume(code);
        return cdataClose;
      }
      if (markdownLineEnding(code)) {
        returnState = cdata;
        return lineEndingBefore(code);
      }
      effects.consume(code);
      return cdata;
    }
    function cdataClose(code) {
      if (code === codes.rightSquareBracket) {
        effects.consume(code);
        return cdataEnd;
      }
      return cdata(code);
    }
    function cdataEnd(code) {
      if (code === codes.greaterThan) {
        return end(code);
      }
      if (code === codes.rightSquareBracket) {
        effects.consume(code);
        return cdataEnd;
      }
      return cdata(code);
    }
    function declaration(code) {
      if (code === codes.eof || code === codes.greaterThan) {
        return end(code);
      }
      if (markdownLineEnding(code)) {
        returnState = declaration;
        return lineEndingBefore(code);
      }
      effects.consume(code);
      return declaration;
    }
    function instruction(code) {
      if (code === codes.eof) {
        return nok(code);
      }
      if (code === codes.questionMark) {
        effects.consume(code);
        return instructionClose;
      }
      if (markdownLineEnding(code)) {
        returnState = instruction;
        return lineEndingBefore(code);
      }
      effects.consume(code);
      return instruction;
    }
    function instructionClose(code) {
      return code === codes.greaterThan ? end(code) : instruction(code);
    }
    function tagCloseStart(code) {
      if (asciiAlpha(code)) {
        effects.consume(code);
        return tagClose;
      }
      return nok(code);
    }
    function tagClose(code) {
      if (code === codes.dash || asciiAlphanumeric(code)) {
        effects.consume(code);
        return tagClose;
      }
      return tagCloseBetween(code);
    }
    function tagCloseBetween(code) {
      if (markdownLineEnding(code)) {
        returnState = tagCloseBetween;
        return lineEndingBefore(code);
      }
      if (markdownSpace(code)) {
        effects.consume(code);
        return tagCloseBetween;
      }
      return end(code);
    }
    function tagOpen(code) {
      if (code === codes.dash || asciiAlphanumeric(code)) {
        effects.consume(code);
        return tagOpen;
      }
      if (code === codes.slash || code === codes.greaterThan || markdownLineEndingOrSpace(code)) {
        return tagOpenBetween(code);
      }
      return nok(code);
    }
    function tagOpenBetween(code) {
      if (code === codes.slash) {
        effects.consume(code);
        return end;
      }
      if (code === codes.colon || code === codes.underscore || asciiAlpha(code)) {
        effects.consume(code);
        return tagOpenAttributeName;
      }
      if (markdownLineEnding(code)) {
        returnState = tagOpenBetween;
        return lineEndingBefore(code);
      }
      if (markdownSpace(code)) {
        effects.consume(code);
        return tagOpenBetween;
      }
      return end(code);
    }
    function tagOpenAttributeName(code) {
      if (code === codes.dash || code === codes.dot || code === codes.colon || code === codes.underscore || asciiAlphanumeric(code)) {
        effects.consume(code);
        return tagOpenAttributeName;
      }
      return tagOpenAttributeNameAfter(code);
    }
    function tagOpenAttributeNameAfter(code) {
      if (code === codes.equalsTo) {
        effects.consume(code);
        return tagOpenAttributeValueBefore;
      }
      if (markdownLineEnding(code)) {
        returnState = tagOpenAttributeNameAfter;
        return lineEndingBefore(code);
      }
      if (markdownSpace(code)) {
        effects.consume(code);
        return tagOpenAttributeNameAfter;
      }
      return tagOpenBetween(code);
    }
    function tagOpenAttributeValueBefore(code) {
      if (code === codes.eof || code === codes.lessThan || code === codes.equalsTo || code === codes.greaterThan || code === codes.graveAccent) {
        return nok(code);
      }
      if (code === codes.quotationMark || code === codes.apostrophe) {
        effects.consume(code);
        marker = code;
        return tagOpenAttributeValueQuoted;
      }
      if (markdownLineEnding(code)) {
        returnState = tagOpenAttributeValueBefore;
        return lineEndingBefore(code);
      }
      if (markdownSpace(code)) {
        effects.consume(code);
        return tagOpenAttributeValueBefore;
      }
      effects.consume(code);
      return tagOpenAttributeValueUnquoted;
    }
    function tagOpenAttributeValueQuoted(code) {
      if (code === marker) {
        effects.consume(code);
        marker = undefined;
        return tagOpenAttributeValueQuotedAfter;
      }
      if (code === codes.eof) {
        return nok(code);
      }
      if (markdownLineEnding(code)) {
        returnState = tagOpenAttributeValueQuoted;
        return lineEndingBefore(code);
      }
      effects.consume(code);
      return tagOpenAttributeValueQuoted;
    }
    function tagOpenAttributeValueUnquoted(code) {
      if (code === codes.eof || code === codes.quotationMark || code === codes.apostrophe || code === codes.lessThan || code === codes.equalsTo || code === codes.graveAccent) {
        return nok(code);
      }
      if (code === codes.slash || code === codes.greaterThan || markdownLineEndingOrSpace(code)) {
        return tagOpenBetween(code);
      }
      effects.consume(code);
      return tagOpenAttributeValueUnquoted;
    }
    function tagOpenAttributeValueQuotedAfter(code) {
      if (code === codes.slash || code === codes.greaterThan || markdownLineEndingOrSpace(code)) {
        return tagOpenBetween(code);
      }
      return nok(code);
    }
    function end(code) {
      if (code === codes.greaterThan) {
        effects.consume(code);
        effects.exit(types.htmlTextData);
        effects.exit(types.htmlText);
        return ok2;
      }
      return nok(code);
    }
    function lineEndingBefore(code) {
      ok(returnState, "expected return state");
      ok(markdownLineEnding(code), "expected eol");
      effects.exit(types.htmlTextData);
      effects.enter(types.lineEnding);
      effects.consume(code);
      effects.exit(types.lineEnding);
      return lineEndingAfter;
    }
    function lineEndingAfter(code) {
      ok(self2.parser.constructs.disable.null, "expected `disable.null` to be populated");
      return markdownSpace(code) ? factorySpace(effects, lineEndingAfterPrefix, types.linePrefix, self2.parser.constructs.disable.null.includes("codeIndented") ? undefined : constants.tabSize)(code) : lineEndingAfterPrefix(code);
    }
    function lineEndingAfterPrefix(code) {
      effects.enter(types.htmlTextData);
      return returnState(code);
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/label-end.js
  var labelEnd = {
    name: "labelEnd",
    resolveAll: resolveAllLabelEnd,
    resolveTo: resolveToLabelEnd,
    tokenize: tokenizeLabelEnd
  };
  var resourceConstruct = { tokenize: tokenizeResource };
  var referenceFullConstruct = { tokenize: tokenizeReferenceFull };
  var referenceCollapsedConstruct = { tokenize: tokenizeReferenceCollapsed };
  function resolveAllLabelEnd(events) {
    let index = -1;
    const newEvents = [];
    while (++index < events.length) {
      const token = events[index][1];
      newEvents.push(events[index]);
      if (token.type === types.labelImage || token.type === types.labelLink || token.type === types.labelEnd) {
        const offset = token.type === types.labelImage ? 4 : 2;
        token.type = types.data;
        index += offset;
      }
    }
    if (events.length !== newEvents.length) {
      splice(events, 0, events.length, newEvents);
    }
    return events;
  }
  function resolveToLabelEnd(events, context) {
    let index = events.length;
    let offset = 0;
    let open;
    let close;
    let media;
    while (index--) {
      const token = events[index][1];
      if (open) {
        if (token.type === types.link || token.type === types.labelLink && token._inactive) {
          break;
        }
        if (events[index][0] === "enter" && token.type === types.labelLink) {
          token._inactive = true;
        }
      } else if (close) {
        if (events[index][0] === "enter" && (token.type === types.labelImage || token.type === types.labelLink) && !token._balanced) {
          open = index;
          if (token.type !== types.labelLink) {
            offset = 2;
            break;
          }
        }
      } else if (token.type === types.labelEnd) {
        close = index;
      }
    }
    ok(open !== undefined, "`open` is supposed to be found");
    ok(close !== undefined, "`close` is supposed to be found");
    const group = {
      type: events[open][1].type === types.labelLink ? types.link : types.image,
      start: { ...events[open][1].start },
      end: { ...events[events.length - 1][1].end }
    };
    const label = {
      type: types.label,
      start: { ...events[open][1].start },
      end: { ...events[close][1].end }
    };
    const text = {
      type: types.labelText,
      start: { ...events[open + offset + 2][1].end },
      end: { ...events[close - 2][1].start }
    };
    media = [
      ["enter", group, context],
      ["enter", label, context]
    ];
    media = push(media, events.slice(open + 1, open + offset + 3));
    media = push(media, [["enter", text, context]]);
    ok(context.parser.constructs.insideSpan.null, "expected `insideSpan.null` to be populated");
    media = push(media, resolveAll(context.parser.constructs.insideSpan.null, events.slice(open + offset + 4, close - 3), context));
    media = push(media, [
      ["exit", text, context],
      events[close - 2],
      events[close - 1],
      ["exit", label, context]
    ]);
    media = push(media, events.slice(close + 1));
    media = push(media, [["exit", group, context]]);
    splice(events, open, events.length, media);
    return events;
  }
  function tokenizeLabelEnd(effects, ok2, nok) {
    const self2 = this;
    const labelStarts = self2._labelStarts;
    let labelStart;
    let defined;
    if (labelStarts) {
      while (labelStarts.length > 0 && labelStarts[labelStarts.length - 1]._balanced) {
        labelStarts.pop();
      }
      labelStart = labelStarts[labelStarts.length - 1];
    }
    return start;
    function start(code) {
      ok(code === codes.rightSquareBracket, "expected `]`");
      if (!labelStart) {
        return nok(code);
      }
      if (labelStart._inactive) {
        return labelEndNok(code);
      }
      defined = self2.parser.defined.includes(normalizeIdentifier(self2.sliceSerialize({ start: labelStart.end, end: self2.now() })));
      effects.enter(types.labelEnd);
      effects.enter(types.labelMarker);
      effects.consume(code);
      effects.exit(types.labelMarker);
      effects.exit(types.labelEnd);
      return after;
    }
    function after(code) {
      if (code === codes.leftParenthesis) {
        return effects.attempt(resourceConstruct, labelEndOk, defined ? labelEndOk : labelEndNok)(code);
      }
      if (code === codes.leftSquareBracket) {
        return effects.attempt(referenceFullConstruct, labelEndOk, defined ? referenceNotFull : labelEndNok)(code);
      }
      return defined ? labelEndOk(code) : labelEndNok(code);
    }
    function referenceNotFull(code) {
      return effects.attempt(referenceCollapsedConstruct, labelEndOk, labelEndNok)(code);
    }
    function labelEndOk(code) {
      ok(labelStarts, "expected `labelStarts` to be populated");
      labelStarts.pop();
      return ok2(code);
    }
    function labelEndNok(code) {
      labelStart._balanced = true;
      return nok(code);
    }
  }
  function tokenizeResource(effects, ok2, nok) {
    return resourceStart;
    function resourceStart(code) {
      ok(code === codes.leftParenthesis, "expected left paren");
      effects.enter(types.resource);
      effects.enter(types.resourceMarker);
      effects.consume(code);
      effects.exit(types.resourceMarker);
      return resourceBefore;
    }
    function resourceBefore(code) {
      return markdownLineEndingOrSpace(code) ? factoryWhitespace(effects, resourceOpen)(code) : resourceOpen(code);
    }
    function resourceOpen(code) {
      if (code === codes.rightParenthesis) {
        return resourceEnd(code);
      }
      return factoryDestination(effects, resourceDestinationAfter, resourceDestinationMissing, types.resourceDestination, types.resourceDestinationLiteral, types.resourceDestinationLiteralMarker, types.resourceDestinationRaw, types.resourceDestinationString, constants.linkResourceDestinationBalanceMax)(code);
    }
    function resourceDestinationAfter(code) {
      return markdownLineEndingOrSpace(code) ? factoryWhitespace(effects, resourceBetween)(code) : resourceEnd(code);
    }
    function resourceDestinationMissing(code) {
      return nok(code);
    }
    function resourceBetween(code) {
      if (code === codes.quotationMark || code === codes.apostrophe || code === codes.leftParenthesis) {
        return factoryTitle(effects, resourceTitleAfter, nok, types.resourceTitle, types.resourceTitleMarker, types.resourceTitleString)(code);
      }
      return resourceEnd(code);
    }
    function resourceTitleAfter(code) {
      return markdownLineEndingOrSpace(code) ? factoryWhitespace(effects, resourceEnd)(code) : resourceEnd(code);
    }
    function resourceEnd(code) {
      if (code === codes.rightParenthesis) {
        effects.enter(types.resourceMarker);
        effects.consume(code);
        effects.exit(types.resourceMarker);
        effects.exit(types.resource);
        return ok2;
      }
      return nok(code);
    }
  }
  function tokenizeReferenceFull(effects, ok2, nok) {
    const self2 = this;
    return referenceFull;
    function referenceFull(code) {
      ok(code === codes.leftSquareBracket, "expected left bracket");
      return factoryLabel.call(self2, effects, referenceFullAfter, referenceFullMissing, types.reference, types.referenceMarker, types.referenceString)(code);
    }
    function referenceFullAfter(code) {
      return self2.parser.defined.includes(normalizeIdentifier(self2.sliceSerialize(self2.events[self2.events.length - 1][1]).slice(1, -1))) ? ok2(code) : nok(code);
    }
    function referenceFullMissing(code) {
      return nok(code);
    }
  }
  function tokenizeReferenceCollapsed(effects, ok2, nok) {
    return referenceCollapsedStart;
    function referenceCollapsedStart(code) {
      ok(code === codes.leftSquareBracket, "expected left bracket");
      effects.enter(types.reference);
      effects.enter(types.referenceMarker);
      effects.consume(code);
      effects.exit(types.referenceMarker);
      return referenceCollapsedOpen;
    }
    function referenceCollapsedOpen(code) {
      if (code === codes.rightSquareBracket) {
        effects.enter(types.referenceMarker);
        effects.consume(code);
        effects.exit(types.referenceMarker);
        effects.exit(types.reference);
        return ok2;
      }
      return nok(code);
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/label-start-image.js
  var labelStartImage = {
    name: "labelStartImage",
    resolveAll: labelEnd.resolveAll,
    tokenize: tokenizeLabelStartImage
  };
  function tokenizeLabelStartImage(effects, ok2, nok) {
    const self2 = this;
    let labelImage;
    return start;
    function start(code) {
      ok(code === codes.exclamationMark, "expected `!`");
      effects.enter(types.labelImage);
      effects.enter(types.labelImageMarker);
      effects.consume(code);
      effects.exit(types.labelImageMarker);
      return open;
    }
    function open(code) {
      if (code === codes.leftSquareBracket) {
        effects.enter(types.labelMarker);
        effects.consume(code);
        effects.exit(types.labelMarker);
        labelImage = effects.exit(types.labelImage);
        return after;
      }
      return nok(code);
    }
    function after(code) {
      if (code === codes.caret && "_hiddenFootnoteSupport" in self2.parser.constructs) {
        return nok(code);
      }
      self2._labelStarts = self2._labelStarts || [];
      self2._labelStarts.push(labelImage);
      return ok2(code);
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/label-start-link.js
  var labelStartLink = {
    name: "labelStartLink",
    resolveAll: labelEnd.resolveAll,
    tokenize: tokenizeLabelStartLink
  };
  function tokenizeLabelStartLink(effects, ok2, nok) {
    const self2 = this;
    let labelLink;
    return start;
    function start(code) {
      ok(code === codes.leftSquareBracket, "expected `[`");
      effects.enter(types.labelLink);
      effects.enter(types.labelMarker);
      effects.consume(code);
      effects.exit(types.labelMarker);
      labelLink = effects.exit(types.labelLink);
      return after;
    }
    function after(code) {
      if (code === codes.caret && "_hiddenFootnoteSupport" in self2.parser.constructs) {
        return nok(code);
      }
      self2._labelStarts = self2._labelStarts || [];
      self2._labelStarts.push(labelLink);
      return ok2(code);
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/line-ending.js
  var lineEnding = { name: "lineEnding", tokenize: tokenizeLineEnding };
  function tokenizeLineEnding(effects, ok2) {
    return start;
    function start(code) {
      ok(markdownLineEnding(code), "expected eol");
      effects.enter(types.lineEnding);
      effects.consume(code);
      effects.exit(types.lineEnding);
      return factorySpace(effects, ok2, types.linePrefix);
    }
  }
  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/thematic-break.js
  var thematicBreak2 = {
    name: "thematicBreak",
    tokenize: tokenizeThematicBreak
  };
  function tokenizeThematicBreak(effects, ok2, nok) {
    let size = 0;
    let marker;
    return start;
    function start(code) {
      effects.enter(types.thematicBreak);
      return before(code);
    }
    function before(code) {
      ok(code === codes.asterisk || code === codes.dash || code === codes.underscore, "expected `*`, `-`, or `_`");
      marker = code;
      return atBreak(code);
    }
    function atBreak(code) {
      if (code === marker) {
        effects.enter(types.thematicBreakSequence);
        return sequence(code);
      }
      if (size >= constants.thematicBreakMarkerCountMin && (code === codes.eof || markdownLineEnding(code))) {
        effects.exit(types.thematicBreak);
        return ok2(code);
      }
      return nok(code);
    }
    function sequence(code) {
      if (code === marker) {
        effects.consume(code);
        size++;
        return sequence;
      }
      effects.exit(types.thematicBreakSequence);
      return markdownSpace(code) ? factorySpace(effects, atBreak, types.whitespace)(code) : atBreak(code);
    }
  }

  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/list.js
  var list2 = {
    continuation: { tokenize: tokenizeListContinuation },
    exit: tokenizeListEnd,
    name: "list",
    tokenize: tokenizeListStart
  };
  var listItemPrefixWhitespaceConstruct = {
    partial: true,
    tokenize: tokenizeListItemPrefixWhitespace
  };
  var indentConstruct = { partial: true, tokenize: tokenizeIndent };
  function tokenizeListStart(effects, ok2, nok) {
    const self2 = this;
    const tail = self2.events[self2.events.length - 1];
    let initialSize = tail && tail[1].type === types.linePrefix ? tail[2].sliceSerialize(tail[1], true).length : 0;
    let size = 0;
    return start;
    function start(code) {
      ok(self2.containerState, "expected state");
      const kind = self2.containerState.type || (code === codes.asterisk || code === codes.plusSign || code === codes.dash ? types.listUnordered : types.listOrdered);
      if (kind === types.listUnordered ? !self2.containerState.marker || code === self2.containerState.marker : asciiDigit(code)) {
        if (!self2.containerState.type) {
          self2.containerState.type = kind;
          effects.enter(kind, { _container: true });
        }
        if (kind === types.listUnordered) {
          effects.enter(types.listItemPrefix);
          return code === codes.asterisk || code === codes.dash ? effects.check(thematicBreak2, nok, atMarker)(code) : atMarker(code);
        }
        if (!self2.interrupt || code === codes.digit1) {
          effects.enter(types.listItemPrefix);
          effects.enter(types.listItemValue);
          return inside(code);
        }
      }
      return nok(code);
    }
    function inside(code) {
      ok(self2.containerState, "expected state");
      if (asciiDigit(code) && ++size < constants.listItemValueSizeMax) {
        effects.consume(code);
        return inside;
      }
      if ((!self2.interrupt || size < 2) && (self2.containerState.marker ? code === self2.containerState.marker : code === codes.rightParenthesis || code === codes.dot)) {
        effects.exit(types.listItemValue);
        return atMarker(code);
      }
      return nok(code);
    }
    function atMarker(code) {
      ok(self2.containerState, "expected state");
      ok(code !== codes.eof, "eof (`null`) is not a marker");
      effects.enter(types.listItemMarker);
      effects.consume(code);
      effects.exit(types.listItemMarker);
      self2.containerState.marker = self2.containerState.marker || code;
      return effects.check(blankLine, self2.interrupt ? nok : onBlank, effects.attempt(listItemPrefixWhitespaceConstruct, endOfPrefix, otherPrefix));
    }
    function onBlank(code) {
      ok(self2.containerState, "expected state");
      self2.containerState.initialBlankLine = true;
      initialSize++;
      return endOfPrefix(code);
    }
    function otherPrefix(code) {
      if (markdownSpace(code)) {
        effects.enter(types.listItemPrefixWhitespace);
        effects.consume(code);
        effects.exit(types.listItemPrefixWhitespace);
        return endOfPrefix;
      }
      return nok(code);
    }
    function endOfPrefix(code) {
      ok(self2.containerState, "expected state");
      self2.containerState.size = initialSize + self2.sliceSerialize(effects.exit(types.listItemPrefix), true).length;
      return ok2(code);
    }
  }
  function tokenizeListContinuation(effects, ok2, nok) {
    const self2 = this;
    ok(self2.containerState, "expected state");
    self2.containerState._closeFlow = undefined;
    return effects.check(blankLine, onBlank, notBlank);
    function onBlank(code) {
      ok(self2.containerState, "expected state");
      ok(typeof self2.containerState.size === "number", "expected size");
      self2.containerState.furtherBlankLines = self2.containerState.furtherBlankLines || self2.containerState.initialBlankLine;
      return factorySpace(effects, ok2, types.listItemIndent, self2.containerState.size + 1)(code);
    }
    function notBlank(code) {
      ok(self2.containerState, "expected state");
      if (self2.containerState.furtherBlankLines || !markdownSpace(code)) {
        self2.containerState.furtherBlankLines = undefined;
        self2.containerState.initialBlankLine = undefined;
        return notInCurrentItem(code);
      }
      self2.containerState.furtherBlankLines = undefined;
      self2.containerState.initialBlankLine = undefined;
      return effects.attempt(indentConstruct, ok2, notInCurrentItem)(code);
    }
    function notInCurrentItem(code) {
      ok(self2.containerState, "expected state");
      self2.containerState._closeFlow = true;
      self2.interrupt = undefined;
      ok(self2.parser.constructs.disable.null, "expected `disable.null` to be populated");
      return factorySpace(effects, effects.attempt(list2, ok2, nok), types.linePrefix, self2.parser.constructs.disable.null.includes("codeIndented") ? undefined : constants.tabSize)(code);
    }
  }
  function tokenizeIndent(effects, ok2, nok) {
    const self2 = this;
    ok(self2.containerState, "expected state");
    ok(typeof self2.containerState.size === "number", "expected size");
    return factorySpace(effects, afterPrefix, types.listItemIndent, self2.containerState.size + 1);
    function afterPrefix(code) {
      ok(self2.containerState, "expected state");
      const tail = self2.events[self2.events.length - 1];
      return tail && tail[1].type === types.listItemIndent && tail[2].sliceSerialize(tail[1], true).length === self2.containerState.size ? ok2(code) : nok(code);
    }
  }
  function tokenizeListEnd(effects) {
    ok(this.containerState, "expected state");
    ok(typeof this.containerState.type === "string", "expected type");
    effects.exit(this.containerState.type);
  }
  function tokenizeListItemPrefixWhitespace(effects, ok2, nok) {
    const self2 = this;
    ok(self2.parser.constructs.disable.null, "expected `disable.null` to be populated");
    return factorySpace(effects, afterPrefix, types.listItemPrefixWhitespace, self2.parser.constructs.disable.null.includes("codeIndented") ? undefined : constants.tabSize + 1);
    function afterPrefix(code) {
      const tail = self2.events[self2.events.length - 1];
      return !markdownSpace(code) && tail && tail[1].type === types.listItemPrefixWhitespace ? ok2(code) : nok(code);
    }
  }
  // ../../node_modules/.bun/micromark-util-edit-map@1.0.0/node_modules/micromark-util-edit-map/dev/index.js
  class EditMap {
    constructor() {
      this.index = new Map;
      this.map = [];
    }
    add(index, remove, add) {
      addImplementation(this, index, remove, add, false);
    }
    addBefore(index, remove, add) {
      addImplementation(this, index, remove, add, true);
    }
    consume(events) {
      this.map.sort(function(a, b) {
        return a[0] - b[0];
      });
      if (this.map.length === 0) {
        return;
      }
      let index = this.map.length;
      const vecs = [];
      while (index > 0) {
        index -= 1;
        vecs.push(events.slice(this.map[index][0] + this.map[index][1]), this.map[index][2]);
        events.length = this.map[index][0];
      }
      vecs.push(events.slice());
      events.length = 0;
      let slice = vecs.pop();
      while (slice) {
        for (const element of slice) {
          events.push(element);
        }
        slice = vecs.pop();
      }
      this.map.length = 0;
      this.index.clear();
    }
  }
  function addImplementation(editMap, at, remove, add, before) {
    if (remove === 0 && add.length === 0) {
      return;
    }
    const existing = editMap.index.get(at);
    if (existing) {
      existing[1] += remove;
      if (before) {
        add.push(...existing[2]);
        existing[2] = add;
      } else {
        existing[2].push(...add);
      }
      return;
    }
    const change = [at, remove, add];
    editMap.map.push(change);
    editMap.index.set(at, change);
  }

  // ../../node_modules/.bun/micromark-core-commonmark@2.0.4/node_modules/micromark-core-commonmark/dev/lib/setext-underline.js
  var setextUnderline = {
    name: "setextUnderline",
    resolveTo: resolveToSetextUnderline,
    tokenize: tokenizeSetextUnderline
  };
  function resolveToSetextUnderline(events, context) {
    const editMap = new EditMap;
    let index = events.length;
    let content;
    let text;
    let definition;
    while (index--) {
      if (events[index][0] === "enter") {
        if (events[index][1].type === types.content) {
          content = index;
          break;
        }
        if (events[index][1].type === types.paragraph) {
          text = index;
        }
      } else {
        if (events[index][1].type === types.content) {
          editMap.add(index, 1, []);
        }
        if (!definition && events[index][1].type === types.definition) {
          definition = index;
        }
      }
    }
    ok(text !== undefined, "expected a `text` index to be found");
    ok(content !== undefined, "expected a `text` index to be found");
    ok(events[content][2] === context, "enter context should be same");
    ok(events[events.length - 1][2] === context, "enter context should be same");
    const heading = {
      type: types.setextHeading,
      start: { ...events[content][1].start },
      end: { ...events[events.length - 1][1].end }
    };
    events[text][1].type = types.setextHeadingText;
    if (definition) {
      editMap.add(text, 0, [["enter", heading, context]]);
      editMap.add(definition + 1, 0, [["exit", events[content][1], context]]);
      events[content][1].end = { ...events[definition][1].end };
    } else {
      events[content][1] = heading;
    }
    editMap.add(events.length, 0, [["exit", heading, context]]);
    editMap.consume(events);
    return events;
  }
  function tokenizeSetextUnderline(effects, ok2, nok) {
    const self2 = this;
    let marker;
    return start;
    function start(code) {
      let index = self2.events.length;
      let paragraph;
      ok(code === codes.dash || code === codes.equalsTo, "expected `=` or `-`");
      while (index--) {
        if (self2.events[index][1].type !== types.lineEnding && self2.events[index][1].type !== types.linePrefix && self2.events[index][1].type !== types.content) {
          paragraph = self2.events[index][1].type === types.paragraph;
          break;
        }
      }
      if (!self2.parser.lazy[self2.now().line] && (self2.interrupt || paragraph)) {
        effects.enter(types.setextHeadingLine);
        marker = code;
        return before(code);
      }
      return nok(code);
    }
    function before(code) {
      effects.enter(types.setextHeadingLineSequence);
      return inside(code);
    }
    function inside(code) {
      if (code === marker) {
        effects.consume(code);
        return inside;
      }
      effects.exit(types.setextHeadingLineSequence);
      return markdownSpace(code) ? factorySpace(effects, after, types.lineSuffix)(code) : after(code);
    }
    function after(code) {
      if (code === codes.eof || markdownLineEnding(code)) {
        effects.exit(types.setextHeadingLine);
        return ok2(code);
      }
      return nok(code);
    }
  }
  // ../../node_modules/.bun/micromark-extension-gfm-footnote@2.1.0/node_modules/micromark-extension-gfm-footnote/dev/lib/syntax.js
  var indent = { tokenize: tokenizeIndent2, partial: true };
  function gfmFootnote() {
    return {
      document: {
        [codes.leftSquareBracket]: {
          name: "gfmFootnoteDefinition",
          tokenize: tokenizeDefinitionStart,
          continuation: { tokenize: tokenizeDefinitionContinuation },
          exit: gfmFootnoteDefinitionEnd
        }
      },
      text: {
        [codes.leftSquareBracket]: {
          name: "gfmFootnoteCall",
          tokenize: tokenizeGfmFootnoteCall
        },
        [codes.rightSquareBracket]: {
          name: "gfmPotentialFootnoteCall",
          add: "after",
          tokenize: tokenizePotentialGfmFootnoteCall,
          resolveTo: resolveToPotentialGfmFootnoteCall
        }
      }
    };
  }
  function tokenizePotentialGfmFootnoteCall(effects, ok2, nok) {
    const self2 = this;
    let index = self2.events.length;
    const defined = self2.parser.gfmFootnotes || (self2.parser.gfmFootnotes = []);
    let labelStart;
    while (index--) {
      const token = self2.events[index][1];
      if (token.type === types.labelImage) {
        labelStart = token;
        break;
      }
      if (token.type === "gfmFootnoteCall" || token.type === types.labelLink || token.type === types.label || token.type === types.image || token.type === types.link) {
        break;
      }
    }
    return start;
    function start(code) {
      ok(code === codes.rightSquareBracket, "expected `]`");
      if (!labelStart || !labelStart._balanced) {
        return nok(code);
      }
      const id = normalizeIdentifier(self2.sliceSerialize({ start: labelStart.end, end: self2.now() }));
      if (id.codePointAt(0) !== codes.caret || !defined.includes(id.slice(1))) {
        return nok(code);
      }
      effects.enter("gfmFootnoteCallLabelMarker");
      effects.consume(code);
      effects.exit("gfmFootnoteCallLabelMarker");
      return ok2(code);
    }
  }
  function resolveToPotentialGfmFootnoteCall(events, context) {
    let index = events.length;
    let labelStart;
    while (index--) {
      if (events[index][1].type === types.labelImage && events[index][0] === "enter") {
        labelStart = events[index][1];
        break;
      }
    }
    ok(labelStart, "expected `labelStart` to resolve");
    events[index + 1][1].type = types.data;
    events[index + 3][1].type = "gfmFootnoteCallLabelMarker";
    const call = {
      type: "gfmFootnoteCall",
      start: Object.assign({}, events[index + 3][1].start),
      end: Object.assign({}, events[events.length - 1][1].end)
    };
    const marker = {
      type: "gfmFootnoteCallMarker",
      start: Object.assign({}, events[index + 3][1].end),
      end: Object.assign({}, events[index + 3][1].end)
    };
    marker.end.column++;
    marker.end.offset++;
    marker.end._bufferIndex++;
    const string = {
      type: "gfmFootnoteCallString",
      start: Object.assign({}, marker.end),
      end: Object.assign({}, events[events.length - 1][1].start)
    };
    const chunk = {
      type: types.chunkString,
      contentType: "string",
      start: Object.assign({}, string.start),
      end: Object.assign({}, string.end)
    };
    const replacement = [
      events[index + 1],
      events[index + 2],
      ["enter", call, context],
      events[index + 3],
      events[index + 4],
      ["enter", marker, context],
      ["exit", marker, context],
      ["enter", string, context],
      ["enter", chunk, context],
      ["exit", chunk, context],
      ["exit", string, context],
      events[events.length - 2],
      events[events.length - 1],
      ["exit", call, context]
    ];
    events.splice(index, events.length - index + 1, ...replacement);
    return events;
  }
  function tokenizeGfmFootnoteCall(effects, ok2, nok) {
    const self2 = this;
    const defined = self2.parser.gfmFootnotes || (self2.parser.gfmFootnotes = []);
    let size = 0;
    let data;
    return start;
    function start(code) {
      ok(code === codes.leftSquareBracket, "expected `[`");
      effects.enter("gfmFootnoteCall");
      effects.enter("gfmFootnoteCallLabelMarker");
      effects.consume(code);
      effects.exit("gfmFootnoteCallLabelMarker");
      return callStart;
    }
    function callStart(code) {
      if (code !== codes.caret)
        return nok(code);
      effects.enter("gfmFootnoteCallMarker");
      effects.consume(code);
      effects.exit("gfmFootnoteCallMarker");
      effects.enter("gfmFootnoteCallString");
      effects.enter("chunkString").contentType = "string";
      return callData;
    }
    function callData(code) {
      if (size > constants.linkReferenceSizeMax || code === codes.rightSquareBracket && !data || code === codes.eof || code === codes.leftSquareBracket || markdownLineEndingOrSpace(code)) {
        return nok(code);
      }
      if (code === codes.rightSquareBracket) {
        effects.exit("chunkString");
        const token = effects.exit("gfmFootnoteCallString");
        if (!defined.includes(normalizeIdentifier(self2.sliceSerialize(token)))) {
          return nok(code);
        }
        effects.enter("gfmFootnoteCallLabelMarker");
        effects.consume(code);
        effects.exit("gfmFootnoteCallLabelMarker");
        effects.exit("gfmFootnoteCall");
        return ok2;
      }
      if (!markdownLineEndingOrSpace(code)) {
        data = true;
      }
      size++;
      effects.consume(code);
      return code === codes.backslash ? callEscape : callData;
    }
    function callEscape(code) {
      if (code === codes.leftSquareBracket || code === codes.backslash || code === codes.rightSquareBracket) {
        effects.consume(code);
        size++;
        return callData;
      }
      return callData(code);
    }
  }
  function tokenizeDefinitionStart(effects, ok2, nok) {
    const self2 = this;
    const defined = self2.parser.gfmFootnotes || (self2.parser.gfmFootnotes = []);
    let identifier;
    let size = 0;
    let data;
    return start;
    function start(code) {
      ok(code === codes.leftSquareBracket, "expected `[`");
      effects.enter("gfmFootnoteDefinition")._container = true;
      effects.enter("gfmFootnoteDefinitionLabel");
      effects.enter("gfmFootnoteDefinitionLabelMarker");
      effects.consume(code);
      effects.exit("gfmFootnoteDefinitionLabelMarker");
      return labelAtMarker;
    }
    function labelAtMarker(code) {
      if (code === codes.caret) {
        effects.enter("gfmFootnoteDefinitionMarker");
        effects.consume(code);
        effects.exit("gfmFootnoteDefinitionMarker");
        effects.enter("gfmFootnoteDefinitionLabelString");
        effects.enter("chunkString").contentType = "string";
        return labelInside;
      }
      return nok(code);
    }
    function labelInside(code) {
      if (size > constants.linkReferenceSizeMax || code === codes.rightSquareBracket && !data || code === codes.eof || code === codes.leftSquareBracket || markdownLineEndingOrSpace(code)) {
        return nok(code);
      }
      if (code === codes.rightSquareBracket) {
        effects.exit("chunkString");
        const token = effects.exit("gfmFootnoteDefinitionLabelString");
        identifier = normalizeIdentifier(self2.sliceSerialize(token));
        effects.enter("gfmFootnoteDefinitionLabelMarker");
        effects.consume(code);
        effects.exit("gfmFootnoteDefinitionLabelMarker");
        effects.exit("gfmFootnoteDefinitionLabel");
        return labelAfter;
      }
      if (!markdownLineEndingOrSpace(code)) {
        data = true;
      }
      size++;
      effects.consume(code);
      return code === codes.backslash ? labelEscape : labelInside;
    }
    function labelEscape(code) {
      if (code === codes.leftSquareBracket || code === codes.backslash || code === codes.rightSquareBracket) {
        effects.consume(code);
        size++;
        return labelInside;
      }
      return labelInside(code);
    }
    function labelAfter(code) {
      if (code === codes.colon) {
        effects.enter("definitionMarker");
        effects.consume(code);
        effects.exit("definitionMarker");
        if (!defined.includes(identifier)) {
          defined.push(identifier);
        }
        return factorySpace(effects, whitespaceAfter, "gfmFootnoteDefinitionWhitespace");
      }
      return nok(code);
    }
    function whitespaceAfter(code) {
      return ok2(code);
    }
  }
  function tokenizeDefinitionContinuation(effects, ok, nok) {
    return effects.check(blankLine, ok, effects.attempt(indent, ok, nok));
  }
  function gfmFootnoteDefinitionEnd(effects) {
    effects.exit("gfmFootnoteDefinition");
  }
  function tokenizeIndent2(effects, ok, nok) {
    const self2 = this;
    return factorySpace(effects, afterPrefix, "gfmFootnoteDefinitionIndent", constants.tabSize + 1);
    function afterPrefix(code) {
      const tail = self2.events[self2.events.length - 1];
      return tail && tail[1].type === "gfmFootnoteDefinitionIndent" && tail[2].sliceSerialize(tail[1], true).length === constants.tabSize ? ok(code) : nok(code);
    }
  }
  // ../../node_modules/.bun/micromark-extension-gfm-strikethrough@2.1.0/node_modules/micromark-extension-gfm-strikethrough/dev/lib/syntax.js
  function gfmStrikethrough(options) {
    const options_ = options || {};
    let single = options_.singleTilde;
    const tokenizer = {
      name: "strikethrough",
      tokenize: tokenizeStrikethrough,
      resolveAll: resolveAllStrikethrough
    };
    if (single === null || single === undefined) {
      single = true;
    }
    return {
      text: { [codes.tilde]: tokenizer },
      insideSpan: { null: [tokenizer] },
      attentionMarkers: { null: [codes.tilde] }
    };
    function resolveAllStrikethrough(events, context) {
      let index = -1;
      while (++index < events.length) {
        if (events[index][0] === "enter" && events[index][1].type === "strikethroughSequenceTemporary" && events[index][1]._close) {
          let open = index;
          while (open--) {
            if (events[open][0] === "exit" && events[open][1].type === "strikethroughSequenceTemporary" && events[open][1]._open && events[index][1].end.offset - events[index][1].start.offset === events[open][1].end.offset - events[open][1].start.offset) {
              events[index][1].type = "strikethroughSequence";
              events[open][1].type = "strikethroughSequence";
              const strikethrough = {
                type: "strikethrough",
                start: Object.assign({}, events[open][1].start),
                end: Object.assign({}, events[index][1].end)
              };
              const text = {
                type: "strikethroughText",
                start: Object.assign({}, events[open][1].end),
                end: Object.assign({}, events[index][1].start)
              };
              const nextEvents = [
                ["enter", strikethrough, context],
                ["enter", events[open][1], context],
                ["exit", events[open][1], context],
                ["enter", text, context]
              ];
              const insideSpan = context.parser.constructs.insideSpan.null;
              if (insideSpan) {
                splice(nextEvents, nextEvents.length, 0, resolveAll(insideSpan, events.slice(open + 1, index), context));
              }
              splice(nextEvents, nextEvents.length, 0, [
                ["exit", text, context],
                ["enter", events[index][1], context],
                ["exit", events[index][1], context],
                ["exit", strikethrough, context]
              ]);
              splice(events, open - 1, index - open + 3, nextEvents);
              index = open + nextEvents.length - 2;
              break;
            }
          }
        }
      }
      index = -1;
      while (++index < events.length) {
        if (events[index][1].type === "strikethroughSequenceTemporary") {
          events[index][1].type = types.data;
        }
      }
      return events;
    }
    function tokenizeStrikethrough(effects, ok2, nok) {
      const previous = this.previous;
      const events = this.events;
      let size = 0;
      return start;
      function start(code) {
        ok(code === codes.tilde, "expected `~`");
        if (previous === codes.tilde && events[events.length - 1][1].type !== types.characterEscape) {
          return nok(code);
        }
        effects.enter("strikethroughSequenceTemporary");
        return more(code);
      }
      function more(code) {
        const before = classifyCharacter(previous);
        if (code === codes.tilde) {
          if (size > 1)
            return nok(code);
          effects.consume(code);
          size++;
          return more;
        }
        if (size < 2 && !single)
          return nok(code);
        const token = effects.exit("strikethroughSequenceTemporary");
        const after = classifyCharacter(code);
        token._open = !after || after === constants.attentionSideAfter && Boolean(before);
        token._close = !before || before === constants.attentionSideAfter && Boolean(after);
        return ok2(code);
      }
    }
  }
  // ../../node_modules/.bun/micromark-extension-gfm-table@2.1.2/node_modules/micromark-extension-gfm-table/dev/lib/edit-map.js
  class EditMap2 {
    constructor() {
      this.map = [];
      this.index = new Map;
    }
    add(index, remove, add) {
      addImplementation2(this, index, remove, add);
    }
    consume(events) {
      this.map.sort(function(a, b) {
        return a[0] - b[0];
      });
      if (this.map.length === 0) {
        return;
      }
      let index = this.map.length;
      const vecs = [];
      while (index > 0) {
        index -= 1;
        vecs.push(events.slice(this.map[index][0] + this.map[index][1]), this.map[index][2]);
        events.length = this.map[index][0];
      }
      vecs.push(events.slice());
      events.length = 0;
      let slice = vecs.pop();
      while (slice) {
        for (const element of slice) {
          events.push(element);
        }
        slice = vecs.pop();
      }
      this.map.length = 0;
      this.index.clear();
    }
  }
  function addImplementation2(editMap, at, remove, add) {
    if (remove === 0 && add.length === 0) {
      return;
    }
    const existing = editMap.index.get(at);
    if (existing) {
      existing[1] += remove;
      existing[2].push(...add);
      return;
    }
    const change = [at, remove, add];
    editMap.map.push(change);
    editMap.index.set(at, change);
  }

  // ../../node_modules/.bun/micromark-extension-gfm-table@2.1.2/node_modules/micromark-extension-gfm-table/dev/lib/infer.js
  function gfmTableAlign(events, index) {
    ok(events[index][1].type === "table", "expected table");
    let inDelimiterRow = false;
    const align = [];
    while (index < events.length) {
      const event = events[index];
      if (inDelimiterRow) {
        if (event[0] === "enter") {
          if (event[1].type === "tableContent") {
            align.push(events[index + 1][1].type === "tableDelimiterMarker" ? "left" : "none");
          }
        } else if (event[1].type === "tableContent") {
          if (events[index - 1][1].type === "tableDelimiterMarker") {
            const alignIndex = align.length - 1;
            align[alignIndex] = align[alignIndex] === "left" ? "center" : "right";
          }
        } else if (event[1].type === "tableDelimiterRow") {
          break;
        }
      } else if (event[0] === "enter" && event[1].type === "tableDelimiterRow") {
        inDelimiterRow = true;
      }
      index += 1;
    }
    return align;
  }

  // ../../node_modules/.bun/micromark-extension-gfm-table@2.1.2/node_modules/micromark-extension-gfm-table/dev/lib/syntax.js
  function gfmTable() {
    return {
      flow: {
        null: { name: "table", tokenize: tokenizeTable, resolveAll: resolveTable }
      }
    };
  }
  function tokenizeTable(effects, ok2, nok) {
    const self2 = this;
    let size = 0;
    let sizeB = 0;
    let seen;
    return start;
    function start(code) {
      let index = self2.events.length - 1;
      while (index > -1) {
        const { type } = self2.events[index][1];
        if (type === types.lineEnding || type === types.linePrefix) {
          index--;
        } else {
          break;
        }
      }
      const tail = index > -1 ? self2.events[index][1].type : null;
      const next = tail === "tableHead" || tail === "tableRow" ? bodyRowStart : headRowBefore;
      if (next === bodyRowStart && self2.parser.lazy[self2.now().line]) {
        return nok(code);
      }
      return next(code);
    }
    function headRowBefore(code) {
      effects.enter("tableHead");
      effects.enter("tableRow");
      return headRowStart(code);
    }
    function headRowStart(code) {
      if (code === codes.verticalBar) {
        return headRowBreak(code);
      }
      seen = true;
      sizeB += 1;
      return headRowBreak(code);
    }
    function headRowBreak(code) {
      if (code === codes.eof) {
        return nok(code);
      }
      if (markdownLineEnding(code)) {
        if (sizeB > 1) {
          sizeB = 0;
          self2.interrupt = true;
          effects.exit("tableRow");
          effects.enter(types.lineEnding);
          effects.consume(code);
          effects.exit(types.lineEnding);
          return headDelimiterStart;
        }
        return nok(code);
      }
      if (markdownSpace(code)) {
        return factorySpace(effects, headRowBreak, types.whitespace)(code);
      }
      sizeB += 1;
      if (seen) {
        seen = false;
        size += 1;
      }
      if (code === codes.verticalBar) {
        effects.enter("tableCellDivider");
        effects.consume(code);
        effects.exit("tableCellDivider");
        seen = true;
        return headRowBreak;
      }
      effects.enter(types.data);
      return headRowData(code);
    }
    function headRowData(code) {
      if (code === codes.eof || code === codes.verticalBar || markdownLineEndingOrSpace(code)) {
        effects.exit(types.data);
        return headRowBreak(code);
      }
      effects.consume(code);
      return code === codes.backslash ? headRowEscape : headRowData;
    }
    function headRowEscape(code) {
      if (code === codes.backslash || code === codes.verticalBar) {
        effects.consume(code);
        return headRowData;
      }
      return headRowData(code);
    }
    function headDelimiterStart(code) {
      self2.interrupt = false;
      if (self2.parser.lazy[self2.now().line]) {
        return nok(code);
      }
      effects.enter("tableDelimiterRow");
      seen = false;
      if (markdownSpace(code)) {
        ok(self2.parser.constructs.disable.null, "expected `disabled.null`");
        return factorySpace(effects, headDelimiterBefore, types.linePrefix, self2.parser.constructs.disable.null.includes("codeIndented") ? undefined : constants.tabSize)(code);
      }
      return headDelimiterBefore(code);
    }
    function headDelimiterBefore(code) {
      if (code === codes.dash || code === codes.colon) {
        return headDelimiterValueBefore(code);
      }
      if (code === codes.verticalBar) {
        seen = true;
        effects.enter("tableCellDivider");
        effects.consume(code);
        effects.exit("tableCellDivider");
        return headDelimiterCellBefore;
      }
      return headDelimiterNok(code);
    }
    function headDelimiterCellBefore(code) {
      if (markdownSpace(code)) {
        return factorySpace(effects, headDelimiterValueBefore, types.whitespace)(code);
      }
      return headDelimiterValueBefore(code);
    }
    function headDelimiterValueBefore(code) {
      if (code === codes.colon) {
        sizeB += 1;
        seen = true;
        effects.enter("tableDelimiterMarker");
        effects.consume(code);
        effects.exit("tableDelimiterMarker");
        return headDelimiterLeftAlignmentAfter;
      }
      if (code === codes.dash) {
        sizeB += 1;
        return headDelimiterLeftAlignmentAfter(code);
      }
      if (code === codes.eof || markdownLineEnding(code)) {
        return headDelimiterCellAfter(code);
      }
      return headDelimiterNok(code);
    }
    function headDelimiterLeftAlignmentAfter(code) {
      if (code === codes.dash) {
        effects.enter("tableDelimiterFiller");
        return headDelimiterFiller(code);
      }
      return headDelimiterNok(code);
    }
    function headDelimiterFiller(code) {
      if (code === codes.dash) {
        effects.consume(code);
        return headDelimiterFiller;
      }
      if (code === codes.colon) {
        seen = true;
        effects.exit("tableDelimiterFiller");
        effects.enter("tableDelimiterMarker");
        effects.consume(code);
        effects.exit("tableDelimiterMarker");
        return headDelimiterRightAlignmentAfter;
      }
      effects.exit("tableDelimiterFiller");
      return headDelimiterRightAlignmentAfter(code);
    }
    function headDelimiterRightAlignmentAfter(code) {
      if (markdownSpace(code)) {
        return factorySpace(effects, headDelimiterCellAfter, types.whitespace)(code);
      }
      return headDelimiterCellAfter(code);
    }
    function headDelimiterCellAfter(code) {
      if (code === codes.verticalBar) {
        return headDelimiterBefore(code);
      }
      if (code === codes.eof || markdownLineEnding(code)) {
        if (!seen || size !== sizeB) {
          return headDelimiterNok(code);
        }
        effects.exit("tableDelimiterRow");
        effects.exit("tableHead");
        return ok2(code);
      }
      return headDelimiterNok(code);
    }
    function headDelimiterNok(code) {
      return nok(code);
    }
    function bodyRowStart(code) {
      effects.enter("tableRow");
      return bodyRowBreak(code);
    }
    function bodyRowBreak(code) {
      if (code === codes.verticalBar) {
        effects.enter("tableCellDivider");
        effects.consume(code);
        effects.exit("tableCellDivider");
        return bodyRowBreak;
      }
      if (code === codes.eof || markdownLineEnding(code)) {
        effects.exit("tableRow");
        return ok2(code);
      }
      if (markdownSpace(code)) {
        return factorySpace(effects, bodyRowBreak, types.whitespace)(code);
      }
      effects.enter(types.data);
      return bodyRowData(code);
    }
    function bodyRowData(code) {
      if (code === codes.eof || code === codes.verticalBar || markdownLineEndingOrSpace(code)) {
        effects.exit(types.data);
        return bodyRowBreak(code);
      }
      effects.consume(code);
      return code === codes.backslash ? bodyRowEscape : bodyRowData;
    }
    function bodyRowEscape(code) {
      if (code === codes.backslash || code === codes.verticalBar) {
        effects.consume(code);
        return bodyRowData;
      }
      return bodyRowData(code);
    }
  }
  function resolveTable(events, context) {
    let index = -1;
    let inFirstCellAwaitingPipe = true;
    let rowKind = 0;
    let lastCell = [0, 0, 0, 0];
    let cell = [0, 0, 0, 0];
    let afterHeadAwaitingFirstBodyRow = false;
    let lastTableEnd = 0;
    let currentTable;
    let currentBody;
    let currentCell;
    const map = new EditMap2;
    while (++index < events.length) {
      const event = events[index];
      const token = event[1];
      if (event[0] === "enter") {
        if (token.type === "tableHead") {
          afterHeadAwaitingFirstBodyRow = false;
          if (lastTableEnd !== 0) {
            ok(currentTable, "there should be a table opening");
            flushTableEnd(map, context, lastTableEnd, currentTable, currentBody);
            currentBody = undefined;
            lastTableEnd = 0;
          }
          currentTable = {
            type: "table",
            start: Object.assign({}, token.start),
            end: Object.assign({}, token.end)
          };
          map.add(index, 0, [["enter", currentTable, context]]);
        } else if (token.type === "tableRow" || token.type === "tableDelimiterRow") {
          inFirstCellAwaitingPipe = true;
          currentCell = undefined;
          lastCell = [0, 0, 0, 0];
          cell = [0, index + 1, 0, 0];
          if (afterHeadAwaitingFirstBodyRow) {
            afterHeadAwaitingFirstBodyRow = false;
            currentBody = {
              type: "tableBody",
              start: Object.assign({}, token.start),
              end: Object.assign({}, token.end)
            };
            map.add(index, 0, [["enter", currentBody, context]]);
          }
          rowKind = token.type === "tableDelimiterRow" ? 2 : currentBody ? 3 : 1;
        } else if (rowKind && (token.type === types.data || token.type === "tableDelimiterMarker" || token.type === "tableDelimiterFiller")) {
          inFirstCellAwaitingPipe = false;
          if (cell[2] === 0) {
            if (lastCell[1] !== 0) {
              cell[0] = cell[1];
              currentCell = flushCell(map, context, lastCell, rowKind, undefined, currentCell);
              lastCell = [0, 0, 0, 0];
            }
            cell[2] = index;
          }
        } else if (token.type === "tableCellDivider") {
          if (inFirstCellAwaitingPipe) {
            inFirstCellAwaitingPipe = false;
          } else {
            if (lastCell[1] !== 0) {
              cell[0] = cell[1];
              currentCell = flushCell(map, context, lastCell, rowKind, undefined, currentCell);
            }
            lastCell = cell;
            cell = [lastCell[1], index, 0, 0];
          }
        }
      } else if (token.type === "tableHead") {
        afterHeadAwaitingFirstBodyRow = true;
        lastTableEnd = index;
      } else if (token.type === "tableRow" || token.type === "tableDelimiterRow") {
        lastTableEnd = index;
        if (lastCell[1] !== 0) {
          cell[0] = cell[1];
          currentCell = flushCell(map, context, lastCell, rowKind, index, currentCell);
        } else if (cell[1] !== 0) {
          currentCell = flushCell(map, context, cell, rowKind, index, currentCell);
        }
        rowKind = 0;
      } else if (rowKind && (token.type === types.data || token.type === "tableDelimiterMarker" || token.type === "tableDelimiterFiller")) {
        cell[3] = index;
      }
    }
    if (lastTableEnd !== 0) {
      ok(currentTable, "expected table opening");
      flushTableEnd(map, context, lastTableEnd, currentTable, currentBody);
    }
    map.consume(context.events);
    index = -1;
    while (++index < context.events.length) {
      const event = context.events[index];
      if (event[0] === "enter" && event[1].type === "table") {
        event[1]._align = gfmTableAlign(context.events, index);
      }
    }
    return events;
  }
  function flushCell(map, context, range, rowKind, rowEnd, previousCell) {
    const groupName = rowKind === 1 ? "tableHeader" : rowKind === 2 ? "tableDelimiter" : "tableData";
    const valueName = "tableContent";
    if (range[0] !== 0) {
      ok(previousCell, "expected previous cell enter");
      previousCell.end = Object.assign({}, getPoint(context.events, range[0]));
      map.add(range[0], 0, [["exit", previousCell, context]]);
    }
    const now = getPoint(context.events, range[1]);
    previousCell = {
      type: groupName,
      start: Object.assign({}, now),
      end: Object.assign({}, now)
    };
    map.add(range[1], 0, [["enter", previousCell, context]]);
    if (range[2] !== 0) {
      const relatedStart = getPoint(context.events, range[2]);
      const relatedEnd = getPoint(context.events, range[3]);
      const valueToken = {
        type: valueName,
        start: Object.assign({}, relatedStart),
        end: Object.assign({}, relatedEnd)
      };
      map.add(range[2], 0, [["enter", valueToken, context]]);
      ok(range[3] !== 0);
      if (rowKind !== 2) {
        const start = context.events[range[2]];
        const end = context.events[range[3]];
        start[1].end = Object.assign({}, end[1].end);
        start[1].type = types.chunkText;
        start[1].contentType = constants.contentTypeText;
        if (range[3] > range[2] + 1) {
          const a = range[2] + 1;
          const b = range[3] - range[2] - 1;
          map.add(a, b, []);
        }
      }
      map.add(range[3] + 1, 0, [["exit", valueToken, context]]);
    }
    if (rowEnd !== undefined) {
      previousCell.end = Object.assign({}, getPoint(context.events, rowEnd));
      map.add(rowEnd, 0, [["exit", previousCell, context]]);
      previousCell = undefined;
    }
    return previousCell;
  }
  function flushTableEnd(map, context, index, table, tableBody) {
    const exits = [];
    const related = getPoint(context.events, index);
    if (tableBody) {
      tableBody.end = Object.assign({}, related);
      exits.push(["exit", tableBody, context]);
    }
    table.end = Object.assign({}, related);
    exits.push(["exit", table, context]);
    map.add(index + 1, 0, exits);
  }
  function getPoint(events, index) {
    const event = events[index];
    const side = event[0] === "enter" ? "start" : "end";
    return event[1][side];
  }
  // ../../node_modules/.bun/micromark-extension-gfm-task-list-item@2.1.0/node_modules/micromark-extension-gfm-task-list-item/dev/lib/syntax.js
  var tasklistCheck = { name: "tasklistCheck", tokenize: tokenizeTasklistCheck };
  function gfmTaskListItem() {
    return {
      text: { [codes.leftSquareBracket]: tasklistCheck }
    };
  }
  function tokenizeTasklistCheck(effects, ok2, nok) {
    const self2 = this;
    return open;
    function open(code) {
      ok(code === codes.leftSquareBracket, "expected `[`");
      if (self2.previous !== codes.eof || !self2._gfmTasklistFirstContentOfListItem) {
        return nok(code);
      }
      effects.enter("taskListCheck");
      effects.enter("taskListCheckMarker");
      effects.consume(code);
      effects.exit("taskListCheckMarker");
      return inside;
    }
    function inside(code) {
      if (markdownLineEndingOrSpace(code)) {
        effects.enter("taskListCheckValueUnchecked");
        effects.consume(code);
        effects.exit("taskListCheckValueUnchecked");
        return close;
      }
      if (code === codes.uppercaseX || code === codes.lowercaseX) {
        effects.enter("taskListCheckValueChecked");
        effects.consume(code);
        effects.exit("taskListCheckValueChecked");
        return close;
      }
      return nok(code);
    }
    function close(code) {
      if (code === codes.rightSquareBracket) {
        effects.enter("taskListCheckMarker");
        effects.consume(code);
        effects.exit("taskListCheckMarker");
        effects.exit("taskListCheck");
        return after;
      }
      return nok(code);
    }
    function after(code) {
      if (markdownLineEnding(code)) {
        return ok2(code);
      }
      if (markdownSpace(code)) {
        return effects.check({ tokenize: spaceThenNonSpace }, ok2, nok)(code);
      }
      return nok(code);
    }
  }
  function spaceThenNonSpace(effects, ok, nok) {
    return factorySpace(effects, after, types.whitespace);
    function after(code) {
      return code === codes.eof ? nok(code) : ok(code);
    }
  }
  // ../../node_modules/.bun/micromark-extension-gfm@3.0.0/node_modules/micromark-extension-gfm/index.js
  function gfm(options) {
    return combineExtensions([
      gfmAutolinkLiteral(),
      gfmFootnote(),
      gfmStrikethrough(options),
      gfmTable(),
      gfmTaskListItem()
    ]);
  }

  // ../../node_modules/.bun/remark-gfm@4.0.1/node_modules/remark-gfm/lib/index.js
  var emptyOptions2 = {};
  function remarkGfm(options) {
    const self2 = this;
    const settings = options || emptyOptions2;
    const data = self2.data();
    const micromarkExtensions = data.micromarkExtensions || (data.micromarkExtensions = []);
    const fromMarkdownExtensions = data.fromMarkdownExtensions || (data.fromMarkdownExtensions = []);
    const toMarkdownExtensions = data.toMarkdownExtensions || (data.toMarkdownExtensions = []);
    micromarkExtensions.push(gfm(settings));
    fromMarkdownExtensions.push(gfmFromMarkdown());
    toMarkdownExtensions.push(gfmToMarkdown(settings));
  }
  // ../../node_modules/.bun/micromark-util-decode-numeric-character-reference@2.0.2/node_modules/micromark-util-decode-numeric-character-reference/dev/index.js
  function decodeNumericCharacterReference(value, base) {
    const code = Number.parseInt(value, base);
    if (code < codes.ht || code === codes.vt || code > codes.cr && code < codes.space || code > codes.tilde && code < 160 || code > 55295 && code < 57344 || code > 64975 && code < 65008 || (code & 65535) === 65535 || (code & 65535) === 65534 || code > 1114111) {
      return values.replacementCharacter;
    }
    return String.fromCodePoint(code);
  }

  // ../../node_modules/.bun/micromark@4.0.3/node_modules/micromark/dev/lib/initialize/content.js
  var content2 = { tokenize: initializeContent };
  function initializeContent(effects) {
    const contentStart = effects.attempt(this.parser.constructs.contentInitial, afterContentStartConstruct, paragraphInitial);
    let previous;
    return contentStart;
    function afterContentStartConstruct(code) {
      ok(code === codes.eof || markdownLineEnding(code), "expected eol or eof");
      if (code === codes.eof) {
        effects.consume(code);
        return;
      }
      effects.enter(types.lineEnding);
      effects.consume(code);
      effects.exit(types.lineEnding);
      return factorySpace(effects, contentStart, types.linePrefix);
    }
    function paragraphInitial(code) {
      ok(code !== codes.eof && !markdownLineEnding(code), "expected anything other than a line ending or EOF");
      effects.enter(types.paragraph);
      return lineStart(code);
    }
    function lineStart(code) {
      const token = effects.enter(types.chunkText, {
        contentType: constants.contentTypeText,
        previous
      });
      if (previous) {
        previous.next = token;
      }
      previous = token;
      return data(code);
    }
    function data(code) {
      if (code === codes.eof) {
        effects.exit(types.chunkText);
        effects.exit(types.paragraph);
        effects.consume(code);
        return;
      }
      if (markdownLineEnding(code)) {
        effects.consume(code);
        effects.exit(types.chunkText);
        return lineStart;
      }
      effects.consume(code);
      return data;
    }
  }

  // ../../node_modules/.bun/micromark@4.0.3/node_modules/micromark/dev/lib/initialize/document.js
  var document2 = { tokenize: initializeDocument };
  var containerConstruct = { tokenize: tokenizeContainer };
  function initializeDocument(effects) {
    const self2 = this;
    const stack = [];
    let continued = 0;
    let childFlow;
    let childToken;
    let lineStartOffset;
    return start;
    function start(code) {
      if (continued < stack.length) {
        const item = stack[continued];
        self2.containerState = item[1];
        ok(item[0].continuation, "expected `continuation` to be defined on container construct");
        return effects.attempt(item[0].continuation, documentContinue, checkNewContainers)(code);
      }
      return checkNewContainers(code);
    }
    function documentContinue(code) {
      ok(self2.containerState, "expected `containerState` to be defined after continuation");
      continued++;
      if (self2.containerState._closeFlow) {
        self2.containerState._closeFlow = undefined;
        if (childFlow) {
          closeFlow();
        }
        const indexBeforeExits = self2.events.length;
        let indexBeforeFlow = indexBeforeExits;
        let point;
        while (indexBeforeFlow--) {
          if (self2.events[indexBeforeFlow][0] === "exit" && self2.events[indexBeforeFlow][1].type === types.chunkFlow) {
            point = self2.events[indexBeforeFlow][1].end;
            break;
          }
        }
        ok(point, "could not find previous flow chunk");
        exitContainers(continued);
        let index = indexBeforeExits;
        while (index < self2.events.length) {
          self2.events[index][1].end = { ...point };
          index++;
        }
        const editMap = new EditMap;
        editMap.add(indexBeforeFlow + 1, 0, self2.events.slice(indexBeforeExits));
        editMap.add(indexBeforeExits, index - indexBeforeExits, []);
        editMap.consume(self2.events);
        return checkNewContainers(code);
      }
      return start(code);
    }
    function checkNewContainers(code) {
      if (continued === stack.length) {
        if (!childFlow) {
          return documentContinued(code);
        }
        if (childFlow.currentConstruct && childFlow.currentConstruct.concrete) {
          return flowStart(code);
        }
        self2.interrupt = Boolean(childFlow.currentConstruct && !childFlow._gfmTableDynamicInterruptHack);
      }
      self2.containerState = {};
      return effects.check(containerConstruct, thereIsANewContainer, thereIsNoNewContainer)(code);
    }
    function thereIsANewContainer(code) {
      if (childFlow) {
        closeFlow();
      }
      exitContainers(continued);
      return documentContinued(code);
    }
    function thereIsNoNewContainer(code) {
      self2.parser.lazy[self2.now().line] = continued !== stack.length;
      lineStartOffset = self2.now().offset;
      return flowStart(code);
    }
    function documentContinued(code) {
      self2.containerState = {};
      return effects.attempt(containerConstruct, containerContinue, flowStart)(code);
    }
    function containerContinue(code) {
      ok(self2.currentConstruct, "expected `currentConstruct` to be defined on tokenizer");
      ok(self2.containerState, "expected `containerState` to be defined on tokenizer");
      continued++;
      stack.push([self2.currentConstruct, self2.containerState]);
      return documentContinued(code);
    }
    function flowStart(code) {
      if (code === codes.eof) {
        if (childFlow) {
          closeFlow();
        }
        exitContainers(0);
        effects.consume(code);
        return;
      }
      childFlow = childFlow || self2.parser.flow(self2.now());
      effects.enter(types.chunkFlow, {
        _tokenizer: childFlow,
        contentType: constants.contentTypeFlow,
        previous: childToken
      });
      return flowContinue(code);
    }
    function flowContinue(code) {
      if (code === codes.eof) {
        writeToChild(effects.exit(types.chunkFlow), true);
        exitContainers(0);
        effects.consume(code);
        return;
      }
      if (markdownLineEnding(code)) {
        effects.consume(code);
        writeToChild(effects.exit(types.chunkFlow));
        continued = 0;
        self2.interrupt = undefined;
        return start;
      }
      effects.consume(code);
      return flowContinue;
    }
    function writeToChild(token, endOfFile) {
      ok(childFlow, "expected `childFlow` to be defined when continuing");
      const stream = self2.sliceStream(token);
      if (endOfFile) {
        stream.push(null);
      }
      token.previous = childToken;
      if (childToken) {
        childToken.next = token;
      }
      childToken = token;
      childFlow.defineSkip(token.start);
      childFlow.write(stream);
      if (self2.parser.lazy[token.start.line]) {
        let index = childFlow.events.length;
        while (index--) {
          if (childFlow.events[index][1].start.offset < lineStartOffset && (!childFlow.events[index][1].end || childFlow.events[index][1].end.offset > lineStartOffset)) {
            return;
          }
        }
        const indexBeforeExits = self2.events.length;
        let indexBeforeFlow = indexBeforeExits;
        let seen;
        let point;
        while (indexBeforeFlow--) {
          if (self2.events[indexBeforeFlow][0] === "exit" && self2.events[indexBeforeFlow][1].type === types.chunkFlow) {
            if (seen) {
              point = self2.events[indexBeforeFlow][1].end;
              break;
            }
            seen = true;
          }
        }
        ok(point, "could not find previous flow chunk");
        exitContainers(continued);
        index = indexBeforeExits;
        while (index < self2.events.length) {
          self2.events[index][1].end = { ...point };
          index++;
        }
        const editMap = new EditMap;
        editMap.add(indexBeforeFlow + 1, 0, self2.events.slice(indexBeforeExits));
        editMap.add(indexBeforeExits, index - indexBeforeExits, []);
        editMap.consume(self2.events);
      }
    }
    function exitContainers(size) {
      let index = stack.length;
      while (index-- > size) {
        const entry = stack[index];
        self2.containerState = entry[1];
        ok(entry[0].exit, "expected `exit` to be defined on container construct");
        entry[0].exit.call(self2, effects);
      }
      stack.length = size;
    }
    function closeFlow() {
      ok(self2.containerState, "expected `containerState` to be defined when closing flow");
      ok(childFlow, "expected `childFlow` to be defined when closing it");
      childFlow.write([codes.eof]);
      childToken = undefined;
      childFlow = undefined;
      self2.containerState._closeFlow = undefined;
    }
  }
  function tokenizeContainer(effects, ok2, nok) {
    ok(this.parser.constructs.disable.null, "expected `disable.null` to be populated");
    return factorySpace(effects, effects.attempt(this.parser.constructs.document, ok2, nok), types.linePrefix, this.parser.constructs.disable.null.includes("codeIndented") ? undefined : constants.tabSize);
  }

  // ../../node_modules/.bun/micromark@4.0.3/node_modules/micromark/dev/lib/initialize/flow.js
  var flow = { tokenize: initializeFlow };
  function initializeFlow(effects) {
    const self2 = this;
    const initial = effects.attempt(blankLine, atBlankEnding, effects.attempt(this.parser.constructs.flowInitial, afterConstruct, factorySpace(effects, effects.attempt(this.parser.constructs.flow, afterConstruct, effects.attempt(content, afterConstruct)), types.linePrefix)));
    return initial;
    function atBlankEnding(code) {
      ok(code === codes.eof || markdownLineEnding(code), "expected eol or eof");
      if (code === codes.eof) {
        effects.consume(code);
        return;
      }
      effects.enter(types.lineEndingBlank);
      effects.consume(code);
      effects.exit(types.lineEndingBlank);
      self2.currentConstruct = undefined;
      return initial;
    }
    function afterConstruct(code) {
      ok(code === codes.eof || markdownLineEnding(code), "expected eol or eof");
      if (code === codes.eof) {
        effects.consume(code);
        return;
      }
      effects.enter(types.lineEnding);
      effects.consume(code);
      effects.exit(types.lineEnding);
      self2.currentConstruct = undefined;
      return initial;
    }
  }

  // ../../node_modules/.bun/micromark@4.0.3/node_modules/micromark/dev/lib/initialize/text.js
  var resolver = { resolveAll: createResolver() };
  var string = initializeFactory("string");
  var text3 = initializeFactory("text");
  function initializeFactory(field) {
    return {
      resolveAll: createResolver(field === "text" ? resolveAllLineSuffixes : undefined),
      tokenize: initializeText
    };
    function initializeText(effects) {
      const self2 = this;
      const constructs = this.parser.constructs[field];
      const text = effects.attempt(constructs, start, notText);
      return start;
      function start(code) {
        return atBreak(code) ? text(code) : notText(code);
      }
      function notText(code) {
        if (code === codes.eof) {
          effects.consume(code);
          return;
        }
        effects.enter(types.data);
        effects.consume(code);
        return data;
      }
      function data(code) {
        if (atBreak(code)) {
          effects.exit(types.data);
          return text(code);
        }
        effects.consume(code);
        return data;
      }
      function atBreak(code) {
        if (code === codes.eof) {
          return true;
        }
        const list = constructs[code];
        let index = -1;
        if (list) {
          ok(Array.isArray(list), "expected `disable.null` to be populated");
          while (++index < list.length) {
            const item = list[index];
            if (!item.previous || item.previous.call(self2, self2.previous)) {
              return true;
            }
          }
        }
        return false;
      }
    }
  }
  function createResolver(extraResolver) {
    return resolveAllText;
    function resolveAllText(events, context) {
      let index = -1;
      let enter;
      while (++index <= events.length) {
        if (enter === undefined) {
          if (events[index] && events[index][1].type === types.data) {
            enter = index;
            index++;
          }
        } else if (!events[index] || events[index][1].type !== types.data) {
          if (index !== enter + 2) {
            events[enter][1].end = events[index - 1][1].end;
            events.splice(enter + 2, index - enter - 2);
            index = enter + 2;
          }
          enter = undefined;
        }
      }
      return extraResolver ? extraResolver(events, context) : events;
    }
  }
  function resolveAllLineSuffixes(events, context) {
    const editMap = new EditMap;
    let eventIndex = 0;
    while (++eventIndex <= events.length) {
      if ((eventIndex === events.length || events[eventIndex][1].type === types.lineEnding) && events[eventIndex - 1][1].type === types.data) {
        const data = events[eventIndex - 1][1];
        const chunks = context.sliceStream(data);
        let index = chunks.length;
        let bufferIndex = -1;
        let size = 0;
        let tabs;
        while (index--) {
          const chunk = chunks[index];
          if (typeof chunk === "string") {
            bufferIndex = chunk.length;
            while (chunk.charCodeAt(bufferIndex - 1) === codes.space) {
              size++;
              bufferIndex--;
            }
            if (bufferIndex) {
              break;
            }
            bufferIndex = -1;
          } else if (chunk === codes.horizontalTab) {
            tabs = true;
            size++;
          } else if (chunk === codes.virtualSpace) {} else {
            index++;
            break;
          }
        }
        if (context._contentTypeTextTrailing && eventIndex === events.length) {
          size = 0;
        }
        if (size) {
          const token = {
            type: eventIndex === events.length || tabs || size < constants.hardBreakPrefixSizeMin ? types.lineSuffix : types.hardBreakTrailing,
            start: {
              _bufferIndex: index ? bufferIndex : data.start._bufferIndex + bufferIndex,
              _index: data.start._index + index,
              line: data.end.line,
              column: data.end.column - size,
              offset: data.end.offset - size
            },
            end: { ...data.end }
          };
          data.end = { ...token.start };
          if (data.start.offset === data.end.offset) {
            Object.assign(data, token);
          } else {
            editMap.add(eventIndex, 0, [
              ["enter", token, context],
              ["exit", token, context]
            ]);
          }
        }
        eventIndex++;
      }
    }
    editMap.consume(events);
    return events;
  }

  // ../../node_modules/.bun/micromark@4.0.3/node_modules/micromark/dev/lib/constructs.js
  var exports_constructs = {};
  __export(exports_constructs, {
    attentionMarkers: () => attentionMarkers,
    contentInitial: () => contentInitial,
    disable: () => disable,
    document: () => document3,
    flow: () => flow2,
    flowInitial: () => flowInitial,
    insideSpan: () => insideSpan,
    string: () => string2,
    text: () => text4
  });
  var document3 = {
    [codes.asterisk]: list2,
    [codes.plusSign]: list2,
    [codes.dash]: list2,
    [codes.digit0]: list2,
    [codes.digit1]: list2,
    [codes.digit2]: list2,
    [codes.digit3]: list2,
    [codes.digit4]: list2,
    [codes.digit5]: list2,
    [codes.digit6]: list2,
    [codes.digit7]: list2,
    [codes.digit8]: list2,
    [codes.digit9]: list2,
    [codes.greaterThan]: blockQuote
  };
  var contentInitial = {
    [codes.leftSquareBracket]: definition2
  };
  var flowInitial = {
    [codes.horizontalTab]: codeIndented,
    [codes.virtualSpace]: codeIndented,
    [codes.space]: codeIndented
  };
  var flow2 = {
    [codes.numberSign]: headingAtx,
    [codes.asterisk]: thematicBreak2,
    [codes.dash]: [setextUnderline, thematicBreak2],
    [codes.lessThan]: htmlFlow,
    [codes.equalsTo]: setextUnderline,
    [codes.underscore]: thematicBreak2,
    [codes.graveAccent]: codeFenced,
    [codes.tilde]: codeFenced
  };
  var string2 = {
    [codes.ampersand]: characterReference,
    [codes.backslash]: characterEscape
  };
  var text4 = {
    [codes.carriageReturn]: lineEnding,
    [codes.lineFeed]: lineEnding,
    [codes.carriageReturnLineFeed]: lineEnding,
    [codes.exclamationMark]: labelStartImage,
    [codes.ampersand]: characterReference,
    [codes.asterisk]: attention,
    [codes.lessThan]: [autolink, htmlText],
    [codes.leftSquareBracket]: labelStartLink,
    [codes.backslash]: [hardBreakEscape, characterEscape],
    [codes.rightSquareBracket]: labelEnd,
    [codes.underscore]: attention,
    [codes.graveAccent]: codeText
  };
  var insideSpan = { null: [attention, resolver] };
  var attentionMarkers = { null: [codes.asterisk, codes.underscore] };
  var disable = { null: [] };

  // ../../node_modules/.bun/micromark@4.0.3/node_modules/micromark/dev/lib/create-tokenizer.js
  var import_debug = __toESM(require_browser(), 1);
  var debug = import_debug.default("micromark");
  function createTokenizer(parser, initialize, from) {
    let point = {
      _bufferIndex: -1,
      _index: 0,
      line: from && from.line || 1,
      column: from && from.column || 1,
      offset: from && from.offset || 0
    };
    const columnStart = {};
    const resolveAllConstructs = [];
    let chunks = [];
    let stack = [];
    let consumed = true;
    const effects = {
      attempt: constructFactory(onsuccessfulconstruct),
      check: constructFactory(onsuccessfulcheck),
      consume,
      enter,
      exit,
      interrupt: constructFactory(onsuccessfulcheck, { interrupt: true })
    };
    const context = {
      code: codes.eof,
      containerState: {},
      defineSkip,
      events: [],
      now,
      parser,
      previous: codes.eof,
      sliceSerialize,
      sliceStream,
      write
    };
    let state = initialize.tokenize.call(context, effects);
    let expectedCode;
    if (initialize.resolveAll) {
      resolveAllConstructs.push(initialize);
    }
    return context;
    function write(slice) {
      chunks = push(chunks, slice);
      main();
      if (chunks[chunks.length - 1] !== codes.eof) {
        return [];
      }
      addResult(initialize, 0);
      context.events = resolveAll(resolveAllConstructs, context.events, context);
      return context.events;
    }
    function sliceSerialize(token, expandTabs) {
      return serializeChunks(sliceStream(token), expandTabs);
    }
    function sliceStream(token) {
      return sliceChunks(chunks, token);
    }
    function now() {
      const { _bufferIndex, _index, line, column, offset } = point;
      return { _bufferIndex, _index, line, column, offset };
    }
    function defineSkip(value) {
      columnStart[value.line] = value.column;
      accountForPotentialSkip();
      debug("position: define skip: `%j`", point);
    }
    function main() {
      while (point._index < chunks.length) {
        const chunk = chunks[point._index];
        if (typeof chunk === "string") {
          const chunkIndex = point._index;
          if (point._bufferIndex < 0) {
            point._bufferIndex = 0;
          }
          while (point._index === chunkIndex && point._bufferIndex < chunk.length) {
            go(chunk.charCodeAt(point._bufferIndex));
          }
        } else {
          go(chunk);
        }
      }
    }
    function go(code) {
      ok(consumed === true, "expected character to be consumed");
      consumed = undefined;
      debug("main: passing `%s` to %s", code, state && state.name);
      expectedCode = code;
      ok(typeof state === "function", "expected state");
      state = state(code);
    }
    function consume(code) {
      ok(code === expectedCode, "expected given code to equal expected code");
      debug("consume: `%s`", code);
      ok(consumed === undefined, "expected code to not have been consumed: this might be because `return x(code)` instead of `return x` was used");
      ok(code === null ? context.events.length === 0 || context.events[context.events.length - 1][0] === "exit" : context.events[context.events.length - 1][0] === "enter", "expected last token to be open");
      if (markdownLineEnding(code)) {
        point.line++;
        point.column = 1;
        point.offset += code === codes.carriageReturnLineFeed ? 2 : 1;
        accountForPotentialSkip();
        debug("position: after eol: `%j`", point);
      } else if (code !== codes.virtualSpace) {
        point.column++;
        point.offset++;
      }
      if (point._bufferIndex < 0) {
        point._index++;
      } else {
        point._bufferIndex++;
        if (point._bufferIndex === chunks[point._index].length) {
          point._bufferIndex = -1;
          point._index++;
        }
      }
      context.previous = code;
      consumed = true;
    }
    function enter(type, fields) {
      const token = fields || {};
      token.type = type;
      token.start = now();
      ok(typeof type === "string", "expected string type");
      ok(type.length > 0, "expected non-empty string");
      debug("enter: `%s`", type);
      context.events.push(["enter", token, context]);
      stack.push(token);
      return token;
    }
    function exit(type) {
      ok(typeof type === "string", "expected string type");
      ok(type.length > 0, "expected non-empty string");
      const token = stack.pop();
      ok(token, "cannot close w/o open tokens");
      token.end = now();
      ok(type === token.type, "expected exit token to match current token");
      ok(!(token.start._index === token.end._index && token.start._bufferIndex === token.end._bufferIndex), "expected non-empty token (`" + type + "`)");
      debug("exit: `%s`", token.type);
      context.events.push(["exit", token, context]);
      return token;
    }
    function onsuccessfulconstruct(construct, info) {
      addResult(construct, info.from);
    }
    function onsuccessfulcheck(_, info) {
      info.restore();
    }
    function constructFactory(onreturn, fields) {
      return hook;
      function hook(constructs, returnState, bogusState) {
        let listOfConstructs;
        let constructIndex;
        let currentConstruct;
        let info;
        return Array.isArray(constructs) ? handleListOfConstructs(constructs) : ("tokenize" in constructs) ? handleListOfConstructs([constructs]) : handleMapOfConstructs(constructs);
        function handleMapOfConstructs(map) {
          return start;
          function start(code) {
            const left = code !== null && map[code];
            const all = code !== null && map.null;
            const list = [
              ...Array.isArray(left) ? left : left ? [left] : [],
              ...Array.isArray(all) ? all : all ? [all] : []
            ];
            return handleListOfConstructs(list)(code);
          }
        }
        function handleListOfConstructs(list) {
          listOfConstructs = list;
          constructIndex = 0;
          if (list.length === 0) {
            ok(bogusState, "expected `bogusState` to be given");
            return bogusState;
          }
          return handleConstruct(list[constructIndex]);
        }
        function handleConstruct(construct) {
          return start;
          function start(code) {
            info = store();
            currentConstruct = construct;
            if (!construct.partial) {
              context.currentConstruct = construct;
            }
            ok(context.parser.constructs.disable.null, "expected `disable.null` to be populated");
            if (construct.name && context.parser.constructs.disable.null.includes(construct.name)) {
              return nok(code);
            }
            return construct.tokenize.call(fields ? Object.assign(Object.create(context), fields) : context, effects, ok2, nok)(code);
          }
        }
        function ok2(code) {
          ok(code === expectedCode, "expected code");
          consumed = true;
          onreturn(currentConstruct, info);
          return returnState;
        }
        function nok(code) {
          ok(code === expectedCode, "expected code");
          consumed = true;
          info.restore();
          if (++constructIndex < listOfConstructs.length) {
            return handleConstruct(listOfConstructs[constructIndex]);
          }
          return bogusState;
        }
      }
    }
    function addResult(construct, from) {
      if (construct.resolveAll && !resolveAllConstructs.includes(construct)) {
        resolveAllConstructs.push(construct);
      }
      if (construct.resolve) {
        splice(context.events, from, context.events.length - from, construct.resolve(context.events.slice(from), context));
      }
      if (construct.resolveTo) {
        context.events = construct.resolveTo(context.events, context);
      }
      ok(construct.partial || context.events.length === 0 || context.events[context.events.length - 1][0] === "exit", "expected last token to end");
    }
    function store() {
      const startPoint = now();
      const startPrevious = context.previous;
      const startCurrentConstruct = context.currentConstruct;
      const startEventsIndex = context.events.length;
      const startStack = Array.from(stack);
      return { from: startEventsIndex, restore };
      function restore() {
        point = startPoint;
        context.previous = startPrevious;
        context.currentConstruct = startCurrentConstruct;
        context.events.length = startEventsIndex;
        stack = startStack;
        accountForPotentialSkip();
        debug("position: restore: `%j`", point);
      }
    }
    function accountForPotentialSkip() {
      if (point.line in columnStart && point.column < 2) {
        point.column = columnStart[point.line];
        point.offset += columnStart[point.line] - 1;
      }
    }
  }
  function sliceChunks(chunks, token) {
    const startIndex = token.start._index;
    const startBufferIndex = token.start._bufferIndex;
    const endIndex = token.end._index;
    const endBufferIndex = token.end._bufferIndex;
    let view;
    if (startIndex === endIndex) {
      ok(endBufferIndex > -1, "expected non-negative end buffer index");
      ok(startBufferIndex > -1, "expected non-negative start buffer index");
      view = [chunks[startIndex].slice(startBufferIndex, endBufferIndex)];
    } else {
      view = chunks.slice(startIndex, endIndex);
      if (startBufferIndex > -1) {
        const head = view[0];
        if (typeof head === "string") {
          view[0] = head.slice(startBufferIndex);
        } else {
          ok(startBufferIndex === 0, "expected `startBufferIndex` to be `0`");
          view.shift();
        }
      }
      if (endBufferIndex > 0) {
        view.push(chunks[endIndex].slice(0, endBufferIndex));
      }
    }
    return view;
  }
  function serializeChunks(chunks, expandTabs) {
    let index = -1;
    const result = [];
    let atTab;
    while (++index < chunks.length) {
      const chunk = chunks[index];
      let value;
      if (typeof chunk === "string") {
        value = chunk;
      } else {
        switch (chunk) {
          case codes.carriageReturn: {
            value = values.cr;
            break;
          }
          case codes.lineFeed: {
            value = values.lf;
            break;
          }
          case codes.carriageReturnLineFeed: {
            value = values.cr + values.lf;
            break;
          }
          case codes.horizontalTab: {
            value = expandTabs ? values.space : values.ht;
            break;
          }
          case codes.virtualSpace: {
            if (!expandTabs && atTab) {
              continue;
            }
            value = values.space;
            break;
          }
          default: {
            ok(typeof chunk === "number", "expected number");
            value = String.fromCharCode(chunk);
          }
        }
      }
      atTab = chunk === codes.horizontalTab;
      result.push(value);
    }
    return result.join("");
  }

  // ../../node_modules/.bun/micromark@4.0.3/node_modules/micromark/dev/lib/parse.js
  function parse2(options) {
    const settings = options || {};
    const constructs = combineExtensions([exports_constructs, ...settings.extensions || []]);
    const parser = {
      constructs,
      content: create(content2),
      defined: [],
      document: create(document2),
      flow: create(flow),
      lazy: {},
      string: create(string),
      text: create(text3)
    };
    return parser;
    function create(initial) {
      return creator;
      function creator(from) {
        return createTokenizer(parser, initial, from);
      }
    }
  }

  // ../../node_modules/.bun/micromark@4.0.3/node_modules/micromark/dev/lib/postprocess.js
  function postprocess(events) {
    while (!subtokenize(events)) {}
    return events;
  }

  // ../../node_modules/.bun/micromark@4.0.3/node_modules/micromark/dev/lib/preprocess.js
  var search = /[\0\t\n\r]/g;
  function preprocess() {
    let column = 1;
    let buffer = "";
    let start = true;
    let atCarriageReturn;
    return preprocessor;
    function preprocessor(value, encoding, end) {
      value = buffer + (typeof value === "string" ? value.toString() : new TextDecoder(encoding || undefined).decode(value));
      const chunks = [];
      let startPosition = 0;
      buffer = "";
      if (start) {
        if (value.charCodeAt(0) === codes.byteOrderMarker) {
          startPosition++;
        }
        start = undefined;
      }
      while (startPosition < value.length) {
        search.lastIndex = startPosition;
        const match = search.exec(value);
        const endPosition = match && match.index !== undefined ? match.index : value.length;
        const code = value.charCodeAt(endPosition);
        if (!match) {
          buffer = value.slice(startPosition);
          break;
        }
        if (code === codes.lf && startPosition === endPosition && atCarriageReturn) {
          chunks.push(codes.carriageReturnLineFeed);
          atCarriageReturn = undefined;
        } else {
          if (atCarriageReturn) {
            chunks.push(codes.carriageReturn);
            atCarriageReturn = undefined;
          }
          if (startPosition < endPosition) {
            chunks.push(value.slice(startPosition, endPosition));
            column += endPosition - startPosition;
          }
          switch (code) {
            case codes.nul: {
              chunks.push(codes.replacementCharacter);
              column++;
              break;
            }
            case codes.ht: {
              const next = Math.ceil(column / constants.tabSize) * constants.tabSize;
              chunks.push(codes.horizontalTab);
              while (column++ < next) {
                chunks.push(codes.virtualSpace);
              }
              break;
            }
            case codes.lf: {
              chunks.push(codes.lineFeed);
              column = 1;
              break;
            }
            default: {
              atCarriageReturn = true;
              column = 1;
            }
          }
        }
        startPosition = endPosition + 1;
      }
      if (end) {
        if (atCarriageReturn) {
          chunks.push(codes.carriageReturn);
        }
        if (buffer) {
          chunks.push(buffer);
        }
        chunks.push(codes.eof);
      }
      return chunks;
    }
  }
  // ../../node_modules/.bun/micromark-util-decode-string@2.0.1/node_modules/micromark-util-decode-string/dev/index.js
  var characterEscapeOrReference = /\\([!-/:-@[-`{-~])|&(#(?:\d{1,7}|x[\da-f]{1,6})|[\da-z]{1,31});/gi;
  function decodeString(value) {
    return value.replace(characterEscapeOrReference, decode2);
  }
  function decode2($0, $1, $2) {
    if ($1) {
      return $1;
    }
    const head = $2.charCodeAt(0);
    if (head === codes.numberSign) {
      const head = $2.charCodeAt(1);
      const hex = head === codes.lowercaseX || head === codes.uppercaseX;
      return decodeNumericCharacterReference($2.slice(hex ? 2 : 1), hex ? constants.numericBaseHexadecimal : constants.numericBaseDecimal);
    }
    return decodeNamedCharacterReference($2) || $0;
  }

  // ../../node_modules/.bun/unist-util-stringify-position@4.0.0/node_modules/unist-util-stringify-position/lib/index.js
  function stringifyPosition(value) {
    if (!value || typeof value !== "object") {
      return "";
    }
    if ("position" in value || "type" in value) {
      return position(value.position);
    }
    if ("start" in value || "end" in value) {
      return position(value);
    }
    if ("line" in value || "column" in value) {
      return point(value);
    }
    return "";
  }
  function point(point) {
    return index(point && point.line) + ":" + index(point && point.column);
  }
  function position(pos) {
    return point(pos && pos.start) + "-" + point(pos && pos.end);
  }
  function index(value) {
    return value && typeof value === "number" ? value : 1;
  }
  // ../../node_modules/.bun/mdast-util-from-markdown@2.0.3/node_modules/mdast-util-from-markdown/dev/lib/index.js
  var own = {}.hasOwnProperty;
  function fromMarkdown(value, encoding, options) {
    if (encoding && typeof encoding === "object") {
      options = encoding;
      encoding = undefined;
    }
    return compiler(options)(postprocess(parse2(options).document().write(preprocess()(value, encoding, true))));
  }
  function compiler(options) {
    const config = {
      transforms: [],
      canContainEols: ["emphasis", "fragment", "heading", "paragraph", "strong"],
      enter: {
        autolink: opener(link),
        autolinkProtocol: onenterdata,
        autolinkEmail: onenterdata,
        atxHeading: opener(heading),
        blockQuote: opener(blockQuote),
        characterEscape: onenterdata,
        characterReference: onenterdata,
        codeFenced: opener(codeFlow),
        codeFencedFenceInfo: buffer,
        codeFencedFenceMeta: buffer,
        codeIndented: opener(codeFlow, buffer),
        codeText: opener(codeText, buffer),
        codeTextData: onenterdata,
        data: onenterdata,
        codeFlowValue: onenterdata,
        definition: opener(definition),
        definitionDestinationString: buffer,
        definitionLabelString: buffer,
        definitionTitleString: buffer,
        emphasis: opener(emphasis),
        hardBreakEscape: opener(hardBreak),
        hardBreakTrailing: opener(hardBreak),
        htmlFlow: opener(html, buffer),
        htmlFlowData: onenterdata,
        htmlText: opener(html, buffer),
        htmlTextData: onenterdata,
        image: opener(image),
        label: buffer,
        link: opener(link),
        listItem: opener(listItem),
        listItemValue: onenterlistitemvalue,
        listOrdered: opener(list, onenterlistordered),
        listUnordered: opener(list),
        paragraph: opener(paragraph),
        reference: onenterreference,
        referenceString: buffer,
        resourceDestinationString: buffer,
        resourceTitleString: buffer,
        setextHeading: opener(heading),
        strong: opener(strong),
        thematicBreak: opener(thematicBreak)
      },
      exit: {
        atxHeading: closer(),
        atxHeadingSequence: onexitatxheadingsequence,
        autolink: closer(),
        autolinkEmail: onexitautolinkemail,
        autolinkProtocol: onexitautolinkprotocol,
        blockQuote: closer(),
        characterEscapeValue: onexitdata,
        characterReferenceMarkerHexadecimal: onexitcharacterreferencemarker,
        characterReferenceMarkerNumeric: onexitcharacterreferencemarker,
        characterReferenceValue: onexitcharacterreferencevalue,
        characterReference: onexitcharacterreference,
        codeFenced: closer(onexitcodefenced),
        codeFencedFence: onexitcodefencedfence,
        codeFencedFenceInfo: onexitcodefencedfenceinfo,
        codeFencedFenceMeta: onexitcodefencedfencemeta,
        codeFlowValue: onexitdata,
        codeIndented: closer(onexitcodeindented),
        codeText: closer(onexitcodetext),
        codeTextData: onexitdata,
        data: onexitdata,
        definition: closer(),
        definitionDestinationString: onexitdefinitiondestinationstring,
        definitionLabelString: onexitdefinitionlabelstring,
        definitionTitleString: onexitdefinitiontitlestring,
        emphasis: closer(),
        hardBreakEscape: closer(onexithardbreak),
        hardBreakTrailing: closer(onexithardbreak),
        htmlFlow: closer(onexithtmlflow),
        htmlFlowData: onexitdata,
        htmlText: closer(onexithtmltext),
        htmlTextData: onexitdata,
        image: closer(onexitimage),
        label: onexitlabel,
        labelText: onexitlabeltext,
        lineEnding: onexitlineending,
        link: closer(onexitlink),
        listItem: closer(),
        listOrdered: closer(),
        listUnordered: closer(),
        paragraph: closer(),
        referenceString: onexitreferencestring,
        resourceDestinationString: onexitresourcedestinationstring,
        resourceTitleString: onexitresourcetitlestring,
        resource: onexitresource,
        setextHeading: closer(onexitsetextheading),
        setextHeadingLineSequence: onexitsetextheadinglinesequence,
        setextHeadingText: onexitsetextheadingtext,
        strong: closer(),
        thematicBreak: closer()
      }
    };
    configure(config, (options || {}).mdastExtensions || []);
    const data = {};
    return compile;
    function compile(events) {
      let tree = { type: "root", children: [] };
      const context = {
        stack: [tree],
        tokenStack: [],
        config,
        enter,
        exit,
        buffer,
        resume,
        data
      };
      const listStack = [];
      let index = -1;
      while (++index < events.length) {
        if (events[index][1].type === types.listOrdered || events[index][1].type === types.listUnordered) {
          if (events[index][0] === "enter") {
            listStack.push(index);
          } else {
            const tail = listStack.pop();
            ok(typeof tail === "number", "expected list to be open");
            index = prepareList(events, tail, index);
          }
        }
      }
      index = -1;
      while (++index < events.length) {
        const handler = config[events[index][0]];
        if (own.call(handler, events[index][1].type)) {
          handler[events[index][1].type].call(Object.assign({ sliceSerialize: events[index][2].sliceSerialize }, context), events[index][1]);
        }
      }
      if (context.tokenStack.length > 0) {
        const tail = context.tokenStack[context.tokenStack.length - 1];
        const handler = tail[1] || defaultOnError;
        handler.call(context, undefined, tail[0]);
      }
      tree.position = {
        start: point2(events.length > 0 ? events[0][1].start : { line: 1, column: 1, offset: 0 }),
        end: point2(events.length > 0 ? events[events.length - 2][1].end : { line: 1, column: 1, offset: 0 })
      };
      index = -1;
      while (++index < config.transforms.length) {
        tree = config.transforms[index](tree) || tree;
      }
      return tree;
    }
    function prepareList(events, start, length) {
      let index = start - 1;
      let containerBalance = -1;
      let listSpread = false;
      let listItem;
      let lineIndex;
      let firstBlankLineIndex;
      let atMarker;
      while (++index <= length) {
        const event = events[index];
        switch (event[1].type) {
          case types.listUnordered:
          case types.listOrdered:
          case types.blockQuote: {
            if (event[0] === "enter") {
              containerBalance++;
            } else {
              containerBalance--;
            }
            atMarker = undefined;
            break;
          }
          case types.lineEndingBlank: {
            if (event[0] === "enter") {
              if (listItem && !atMarker && !containerBalance && !firstBlankLineIndex) {
                firstBlankLineIndex = index;
              }
              atMarker = undefined;
            }
            break;
          }
          case types.linePrefix:
          case types.listItemValue:
          case types.listItemMarker:
          case types.listItemPrefix:
          case types.listItemPrefixWhitespace: {
            break;
          }
          default: {
            atMarker = undefined;
          }
        }
        if (!containerBalance && event[0] === "enter" && event[1].type === types.listItemPrefix || containerBalance === -1 && event[0] === "exit" && (event[1].type === types.listUnordered || event[1].type === types.listOrdered)) {
          if (listItem) {
            let tailIndex = index;
            lineIndex = undefined;
            while (tailIndex--) {
              const tailEvent = events[tailIndex];
              if (tailEvent[1].type === types.lineEnding || tailEvent[1].type === types.lineEndingBlank) {
                if (tailEvent[0] === "exit")
                  continue;
                if (lineIndex) {
                  events[lineIndex][1].type = types.lineEndingBlank;
                  listSpread = true;
                }
                tailEvent[1].type = types.lineEnding;
                lineIndex = tailIndex;
              } else if (tailEvent[1].type === types.linePrefix || tailEvent[1].type === types.blockQuotePrefix || tailEvent[1].type === types.blockQuotePrefixWhitespace || tailEvent[1].type === types.blockQuoteMarker || tailEvent[1].type === types.listItemIndent) {} else {
                break;
              }
            }
            if (firstBlankLineIndex && (!lineIndex || firstBlankLineIndex < lineIndex)) {
              listItem._spread = true;
            }
            listItem.end = Object.assign({}, lineIndex ? events[lineIndex][1].start : event[1].end);
            events.splice(lineIndex || index, 0, ["exit", listItem, event[2]]);
            index++;
            length++;
          }
          if (event[1].type === types.listItemPrefix) {
            const item = {
              type: "listItem",
              _spread: false,
              start: Object.assign({}, event[1].start),
              end: undefined
            };
            listItem = item;
            events.splice(index, 0, ["enter", item, event[2]]);
            index++;
            length++;
            firstBlankLineIndex = undefined;
            atMarker = true;
          }
        }
      }
      events[start][1]._spread = listSpread;
      return length;
    }
    function opener(create, and) {
      return open;
      function open(token) {
        enter.call(this, create(token), token);
        if (and)
          and.call(this, token);
      }
    }
    function buffer() {
      this.stack.push({ type: "fragment", children: [] });
    }
    function enter(node, token, errorHandler) {
      const parent = this.stack[this.stack.length - 1];
      ok(parent, "expected `parent`");
      ok("children" in parent, "expected `parent`");
      const siblings = parent.children;
      siblings.push(node);
      this.stack.push(node);
      this.tokenStack.push([token, errorHandler || undefined]);
      node.position = {
        start: point2(token.start),
        end: undefined
      };
    }
    function closer(and) {
      return close;
      function close(token) {
        if (and)
          and.call(this, token);
        exit.call(this, token);
      }
    }
    function exit(token, onExitError) {
      const node = this.stack.pop();
      ok(node, "expected `node`");
      const open = this.tokenStack.pop();
      if (!open) {
        throw new Error("Cannot close `" + token.type + "` (" + stringifyPosition({ start: token.start, end: token.end }) + "): it’s not open");
      } else if (open[0].type !== token.type) {
        if (onExitError) {
          onExitError.call(this, token, open[0]);
        } else {
          const handler = open[1] || defaultOnError;
          handler.call(this, token, open[0]);
        }
      }
      ok(node.type !== "fragment", "unexpected fragment `exit`ed");
      ok(node.position, "expected `position` to be defined");
      node.position.end = point2(token.end);
    }
    function resume() {
      return toString(this.stack.pop());
    }
    function onenterlistordered() {
      this.data.expectingFirstListItemValue = true;
    }
    function onenterlistitemvalue(token) {
      if (this.data.expectingFirstListItemValue) {
        const ancestor = this.stack[this.stack.length - 2];
        ok(ancestor, "expected nodes on stack");
        ok(ancestor.type === "list", "expected list on stack");
        ancestor.start = Number.parseInt(this.sliceSerialize(token), constants.numericBaseDecimal);
        this.data.expectingFirstListItemValue = undefined;
      }
    }
    function onexitcodefencedfenceinfo() {
      const data = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "code", "expected code on stack");
      node.lang = data;
    }
    function onexitcodefencedfencemeta() {
      const data = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "code", "expected code on stack");
      node.meta = data;
    }
    function onexitcodefencedfence() {
      if (this.data.flowCodeInside)
        return;
      this.buffer();
      this.data.flowCodeInside = true;
    }
    function onexitcodefenced() {
      const data = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "code", "expected code on stack");
      node.value = data.replace(/^(\r?\n|\r)|(\r?\n|\r)$/g, "");
      this.data.flowCodeInside = undefined;
    }
    function onexitcodeindented() {
      const data = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "code", "expected code on stack");
      node.value = data.replace(/(\r?\n|\r)$/g, "");
    }
    function onexitdefinitionlabelstring(token) {
      const label = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "definition", "expected definition on stack");
      node.label = label;
      node.identifier = normalizeIdentifier(this.sliceSerialize(token)).toLowerCase();
    }
    function onexitdefinitiontitlestring() {
      const data = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "definition", "expected definition on stack");
      node.title = data;
    }
    function onexitdefinitiondestinationstring() {
      const data = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "definition", "expected definition on stack");
      node.url = data;
    }
    function onexitatxheadingsequence(token) {
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "heading", "expected heading on stack");
      if (!node.depth) {
        const depth = this.sliceSerialize(token).length;
        ok(depth === 1 || depth === 2 || depth === 3 || depth === 4 || depth === 5 || depth === 6, "expected `depth` between `1` and `6`");
        node.depth = depth;
      }
    }
    function onexitsetextheadingtext() {
      this.data.setextHeadingSlurpLineEnding = true;
    }
    function onexitsetextheadinglinesequence(token) {
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "heading", "expected heading on stack");
      node.depth = this.sliceSerialize(token).codePointAt(0) === codes.equalsTo ? 1 : 2;
    }
    function onexitsetextheading() {
      this.data.setextHeadingSlurpLineEnding = undefined;
    }
    function onenterdata(token) {
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok("children" in node, "expected parent on stack");
      const siblings = node.children;
      let tail = siblings[siblings.length - 1];
      if (!tail || tail.type !== "text") {
        tail = text();
        tail.position = {
          start: point2(token.start),
          end: undefined
        };
        siblings.push(tail);
      }
      this.stack.push(tail);
    }
    function onexitdata(token) {
      const tail = this.stack.pop();
      ok(tail, "expected a `node` to be on the stack");
      ok("value" in tail, "expected a `literal` to be on the stack");
      ok(tail.position, "expected `node` to have an open position");
      tail.value += this.sliceSerialize(token);
      tail.position.end = point2(token.end);
    }
    function onexitlineending(token) {
      const context = this.stack[this.stack.length - 1];
      ok(context, "expected `node`");
      if (this.data.atHardBreak) {
        ok("children" in context, "expected `parent`");
        const tail = context.children[context.children.length - 1];
        ok(tail.position, "expected tail to have a starting position");
        tail.position.end = point2(token.end);
        this.data.atHardBreak = undefined;
        return;
      }
      if (!this.data.setextHeadingSlurpLineEnding && config.canContainEols.includes(context.type)) {
        onenterdata.call(this, token);
        onexitdata.call(this, token);
      }
    }
    function onexithardbreak() {
      this.data.atHardBreak = true;
    }
    function onexithtmlflow() {
      const data = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "html", "expected html on stack");
      node.value = data;
    }
    function onexithtmltext() {
      const data = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "html", "expected html on stack");
      node.value = data;
    }
    function onexitcodetext() {
      const data = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "inlineCode", "expected inline code on stack");
      node.value = data;
    }
    function onexitlink() {
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "link", "expected link on stack");
      if (this.data.inReference) {
        const referenceType = this.data.referenceType || "shortcut";
        node.type += "Reference";
        node.referenceType = referenceType;
        delete node.url;
        delete node.title;
      } else {
        delete node.identifier;
        delete node.label;
      }
      this.data.referenceType = undefined;
    }
    function onexitimage() {
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "image", "expected image on stack");
      if (this.data.inReference) {
        const referenceType = this.data.referenceType || "shortcut";
        node.type += "Reference";
        node.referenceType = referenceType;
        delete node.url;
        delete node.title;
      } else {
        delete node.identifier;
        delete node.label;
      }
      this.data.referenceType = undefined;
    }
    function onexitlabeltext(token) {
      const string = this.sliceSerialize(token);
      const ancestor = this.stack[this.stack.length - 2];
      ok(ancestor, "expected ancestor on stack");
      ok(ancestor.type === "image" || ancestor.type === "link", "expected image or link on stack");
      ancestor.label = decodeString(string);
      ancestor.identifier = normalizeIdentifier(string).toLowerCase();
    }
    function onexitlabel() {
      const fragment = this.stack[this.stack.length - 1];
      ok(fragment, "expected node on stack");
      ok(fragment.type === "fragment", "expected fragment on stack");
      const value = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "image" || node.type === "link", "expected image or link on stack");
      this.data.inReference = true;
      if (node.type === "link") {
        const children = fragment.children;
        node.children = children;
      } else {
        node.alt = value;
      }
    }
    function onexitresourcedestinationstring() {
      const data = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "image" || node.type === "link", "expected image or link on stack");
      node.url = data;
    }
    function onexitresourcetitlestring() {
      const data = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "image" || node.type === "link", "expected image or link on stack");
      node.title = data;
    }
    function onexitresource() {
      this.data.inReference = undefined;
    }
    function onenterreference() {
      this.data.referenceType = "collapsed";
    }
    function onexitreferencestring(token) {
      const label = this.resume();
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "image" || node.type === "link", "expected image reference or link reference on stack");
      node.label = label;
      node.identifier = normalizeIdentifier(this.sliceSerialize(token)).toLowerCase();
      this.data.referenceType = "full";
    }
    function onexitcharacterreferencemarker(token) {
      ok(token.type === "characterReferenceMarkerNumeric" || token.type === "characterReferenceMarkerHexadecimal");
      this.data.characterReferenceType = token.type;
    }
    function onexitcharacterreferencevalue(token) {
      const data = this.sliceSerialize(token);
      const type = this.data.characterReferenceType;
      let value;
      if (type) {
        value = decodeNumericCharacterReference(data, type === types.characterReferenceMarkerNumeric ? constants.numericBaseDecimal : constants.numericBaseHexadecimal);
        this.data.characterReferenceType = undefined;
      } else {
        const result = decodeNamedCharacterReference(data);
        ok(result !== false, "expected reference to decode");
        value = result;
      }
      const tail = this.stack[this.stack.length - 1];
      ok(tail, "expected `node`");
      ok("value" in tail, "expected `node.value`");
      tail.value += value;
    }
    function onexitcharacterreference(token) {
      const tail = this.stack.pop();
      ok(tail, "expected `node`");
      ok(tail.position, "expected `node.position`");
      tail.position.end = point2(token.end);
    }
    function onexitautolinkprotocol(token) {
      onexitdata.call(this, token);
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "link", "expected link on stack");
      node.url = this.sliceSerialize(token);
    }
    function onexitautolinkemail(token) {
      onexitdata.call(this, token);
      const node = this.stack[this.stack.length - 1];
      ok(node, "expected node on stack");
      ok(node.type === "link", "expected link on stack");
      node.url = "mailto:" + this.sliceSerialize(token);
    }
    function blockQuote() {
      return { type: "blockquote", children: [] };
    }
    function codeFlow() {
      return { type: "code", lang: null, meta: null, value: "" };
    }
    function codeText() {
      return { type: "inlineCode", value: "" };
    }
    function definition() {
      return {
        type: "definition",
        identifier: "",
        label: null,
        title: null,
        url: ""
      };
    }
    function emphasis() {
      return { type: "emphasis", children: [] };
    }
    function heading() {
      return {
        type: "heading",
        depth: 0,
        children: []
      };
    }
    function hardBreak() {
      return { type: "break" };
    }
    function html() {
      return { type: "html", value: "" };
    }
    function image() {
      return { type: "image", title: null, url: "", alt: null };
    }
    function link() {
      return { type: "link", title: null, url: "", children: [] };
    }
    function list(token) {
      return {
        type: "list",
        ordered: token.type === "listOrdered",
        start: null,
        spread: token._spread,
        children: []
      };
    }
    function listItem(token) {
      return {
        type: "listItem",
        spread: token._spread,
        checked: null,
        children: []
      };
    }
    function paragraph() {
      return { type: "paragraph", children: [] };
    }
    function strong() {
      return { type: "strong", children: [] };
    }
    function text() {
      return { type: "text", value: "" };
    }
    function thematicBreak() {
      return { type: "thematicBreak" };
    }
  }
  function point2(d) {
    return { line: d.line, column: d.column, offset: d.offset };
  }
  function configure(combined, extensions) {
    let index = -1;
    while (++index < extensions.length) {
      const value = extensions[index];
      if (Array.isArray(value)) {
        configure(combined, value);
      } else {
        extension(combined, value);
      }
    }
  }
  function extension(combined, extension) {
    let key;
    for (key in extension) {
      if (own.call(extension, key)) {
        switch (key) {
          case "canContainEols": {
            const right = extension[key];
            if (right) {
              combined[key].push(...right);
            }
            break;
          }
          case "transforms": {
            const right = extension[key];
            if (right) {
              combined[key].push(...right);
            }
            break;
          }
          case "enter":
          case "exit": {
            const right = extension[key];
            if (right) {
              Object.assign(combined[key], right);
            }
            break;
          }
        }
      }
    }
  }
  function defaultOnError(left, right) {
    if (left) {
      throw new Error("Cannot close `" + left.type + "` (" + stringifyPosition({ start: left.start, end: left.end }) + "): a different token (`" + right.type + "`, " + stringifyPosition({ start: right.start, end: right.end }) + ") is open");
    } else {
      throw new Error("Cannot close document, a token (`" + right.type + "`, " + stringifyPosition({ start: right.start, end: right.end }) + ") is still open");
    }
  }
  // ../../node_modules/.bun/remark-parse@11.0.0/node_modules/remark-parse/lib/index.js
  function remarkParse(options) {
    const self2 = this;
    self2.parser = parser;
    function parser(doc) {
      return fromMarkdown(doc, {
        ...self2.data("settings"),
        ...options,
        extensions: self2.data("micromarkExtensions") || [],
        mdastExtensions: self2.data("fromMarkdownExtensions") || []
      });
    }
  }
  // ../../node_modules/.bun/@ungap+structured-clone@1.4.0/node_modules/@ungap/structured-clone/esm/types.js
  var VOID = -1;
  var PRIMITIVE = 0;
  var ARRAY = 1;
  var OBJECT = 2;
  var DATE = 3;
  var REGEXP = 4;
  var MAP = 5;
  var SET = 6;
  var ERROR = 7;
  var BIGINT = 8;

  // ../../node_modules/.bun/@ungap+structured-clone@1.4.0/node_modules/@ungap/structured-clone/esm/deserialize.js
  var { defineProperty } = Object;
  var env = typeof self === "object" ? self : globalThis;
  var guard = (name, init) => {
    switch (name) {
      case "Function":
      case "SharedWorker":
      case "Worker":
      case "eval":
      case "setInterval":
      case "setTimeout":
        throw new TypeError("unable to deserialize " + name);
    }
    return new env[name](init);
  };
  var deserializer = ($, _) => {
    const as = (out, index) => {
      $.set(index, out);
      return out;
    };
    const unpair = (index) => {
      if ($.has(index))
        return $.get(index);
      const [type, value] = _[index];
      switch (type) {
        case PRIMITIVE:
        case VOID:
          return as(value, index);
        case ARRAY: {
          const arr = as([], index);
          for (const index of value)
            arr.push(unpair(index));
          return arr;
        }
        case OBJECT: {
          const object = as({}, index);
          for (const [key, index] of value) {
            const k = unpair(key), value = unpair(index);
            if (k === "__proto__")
              defineProperty(object, k, {
                value,
                configurable: true,
                enumerable: true,
                writable: true
              });
            else
              object[k] = value;
          }
          return object;
        }
        case DATE:
          return as(new Date(value), index);
        case REGEXP: {
          const { source, flags } = value;
          return as(new RegExp(source, flags), index);
        }
        case MAP: {
          const map = as(new Map, index);
          for (const [key, index] of value)
            map.set(unpair(key), unpair(index));
          return map;
        }
        case SET: {
          const set = as(new Set, index);
          for (const index of value)
            set.add(unpair(index));
          return set;
        }
        case ERROR: {
          const { name, message } = value;
          return as(typeof env[name] === "function" ? guard(name, message) : new Error(message), index);
        }
        case BIGINT:
          return as(BigInt(value), index);
        case "BigInt":
          return as(Object(BigInt(value)), index);
        case "ArrayBuffer":
          return as(new Uint8Array(value).buffer, value);
        case "DataView": {
          const { buffer } = new Uint8Array(value);
          return as(new DataView(buffer), value);
        }
        case "-0":
          return -0;
      }
      return as(guard(type, value), index);
    };
    return unpair;
  };
  var deserialize = (serialized) => deserializer(new Map, serialized)(0);

  // ../../node_modules/.bun/@ungap+structured-clone@1.4.0/node_modules/@ungap/structured-clone/esm/serialize.js
  var EMPTY = "";
  var { toString: toString2 } = {};
  var { keys, is: is2 } = Object;
  var typeOf2 = (value) => {
    const type = typeof value;
    if (type !== "object" || !value)
      return [PRIMITIVE, type];
    const asString = toString2.call(value).slice(8, -1);
    switch (asString) {
      case "Array":
        return [ARRAY, EMPTY];
      case "Object":
        return [OBJECT, EMPTY];
      case "Date":
        return [DATE, EMPTY];
      case "RegExp":
        return [REGEXP, EMPTY];
      case "Map":
        return [MAP, EMPTY];
      case "Set":
        return [SET, EMPTY];
      case "DataView":
        return [ARRAY, asString];
    }
    if (asString.includes("Array"))
      return [ARRAY, asString];
    if (value instanceof Error)
      return [ERROR, value.name || "Error"];
    return [OBJECT, asString];
  };
  var shouldSkip = ([TYPE, type]) => TYPE === PRIMITIVE && (type === "function" || type === "symbol");
  var serializer = (strict, json, $, _) => {
    const as = (out, value) => {
      const index = _.push(out) - 1;
      $.set(value, index);
      return index;
    };
    const pair = (value) => {
      if ($.has(value))
        return $.get(value);
      let [TYPE, type] = typeOf2(value);
      switch (TYPE) {
        case PRIMITIVE: {
          let entry = value;
          switch (type) {
            case "bigint":
              TYPE = BIGINT;
              entry = value.toString();
              break;
            case "number":
              if (!value && is2(value, -0))
                return _.push(["-0"]) - 1;
              break;
            case "function":
            case "symbol":
              if (strict)
                throw new TypeError("unable to serialize " + type);
              entry = null;
              break;
            case "undefined":
              return as([VOID], value);
          }
          return as([TYPE, entry], value);
        }
        case ARRAY: {
          if (type) {
            let spread = value;
            if (type === "DataView") {
              spread = new Uint8Array(value.buffer);
            } else if (type === "ArrayBuffer") {
              spread = new Uint8Array(value);
            }
            return as([type, [...spread]], value);
          }
          const arr = [];
          const index = as([TYPE, arr], value);
          for (const entry of value)
            arr.push(pair(entry));
          return index;
        }
        case OBJECT: {
          if (type) {
            switch (type) {
              case "BigInt":
                return as([type, value.toString()], value);
              case "Boolean":
              case "Number":
              case "String":
                return as([type, value.valueOf()], value);
            }
          }
          if (json && "toJSON" in value)
            return pair(value.toJSON());
          const entries = [];
          const index = as([TYPE, entries], value);
          for (const key of keys(value)) {
            if (strict || !shouldSkip(typeOf2(value[key])))
              entries.push([pair(key), pair(value[key])]);
          }
          return index;
        }
        case DATE:
          return as([TYPE, isNaN(value.getTime()) ? EMPTY : value.toISOString()], value);
        case REGEXP: {
          const { source, flags } = value;
          return as([TYPE, { source, flags }], value);
        }
        case MAP: {
          const entries = [];
          const index = as([TYPE, entries], value);
          for (const [key, entry] of value) {
            if (strict || !(shouldSkip(typeOf2(key)) || shouldSkip(typeOf2(entry))))
              entries.push([pair(key), pair(entry)]);
          }
          return index;
        }
        case SET: {
          const entries = [];
          const index = as([TYPE, entries], value);
          for (const entry of value) {
            if (strict || !shouldSkip(typeOf2(entry)))
              entries.push(pair(entry));
          }
          return index;
        }
      }
      const { message } = value;
      return as([TYPE, { name: type, message }], value);
    };
    return pair;
  };
  var serialize2 = (value, { json, lossy } = {}) => {
    const _ = [];
    return serializer(!(json || lossy), !!json, new Map, _)(value), _;
  };

  // ../../node_modules/.bun/@ungap+structured-clone@1.4.0/node_modules/@ungap/structured-clone/esm/index.js
  var esm_default = typeof structuredClone === "function" ? (any, options) => options && (("json" in options) || ("lossy" in options)) ? deserialize(serialize2(any, options)) : structuredClone(any) : (any, options) => deserialize(serialize2(any, options));

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/footer.js
  function defaultFootnoteBackContent(_, rereferenceIndex) {
    const result = [{ type: "text", value: "↩" }];
    if (rereferenceIndex > 1) {
      result.push({
        type: "element",
        tagName: "sup",
        properties: {},
        children: [{ type: "text", value: String(rereferenceIndex) }]
      });
    }
    return result;
  }
  function defaultFootnoteBackLabel(referenceIndex, rereferenceIndex) {
    return "Back to reference " + (referenceIndex + 1) + (rereferenceIndex > 1 ? "-" + rereferenceIndex : "");
  }
  function footer(state) {
    const clobberPrefix = typeof state.options.clobberPrefix === "string" ? state.options.clobberPrefix : "user-content-";
    const footnoteBackContent = state.options.footnoteBackContent || defaultFootnoteBackContent;
    const footnoteBackLabel = state.options.footnoteBackLabel || defaultFootnoteBackLabel;
    const footnoteLabel = state.options.footnoteLabel || "Footnotes";
    const footnoteLabelTagName = state.options.footnoteLabelTagName || "h2";
    const footnoteLabelProperties = state.options.footnoteLabelProperties || {
      className: ["sr-only"]
    };
    const listItems = [];
    let referenceIndex = -1;
    while (++referenceIndex < state.footnoteOrder.length) {
      const definition = state.footnoteById.get(state.footnoteOrder[referenceIndex]);
      if (!definition) {
        continue;
      }
      const content = state.all(definition);
      const id = String(definition.identifier).toUpperCase();
      const safeId = normalizeUri(id.toLowerCase());
      let rereferenceIndex = 0;
      const backReferences = [];
      const counts = state.footnoteCounts.get(id);
      while (counts !== undefined && ++rereferenceIndex <= counts) {
        if (backReferences.length > 0) {
          backReferences.push({ type: "text", value: " " });
        }
        let children = typeof footnoteBackContent === "string" ? footnoteBackContent : footnoteBackContent(referenceIndex, rereferenceIndex);
        if (typeof children === "string") {
          children = { type: "text", value: children };
        }
        backReferences.push({
          type: "element",
          tagName: "a",
          properties: {
            href: "#" + clobberPrefix + "fnref-" + safeId + (rereferenceIndex > 1 ? "-" + rereferenceIndex : ""),
            dataFootnoteBackref: "",
            ariaLabel: typeof footnoteBackLabel === "string" ? footnoteBackLabel : footnoteBackLabel(referenceIndex, rereferenceIndex),
            className: ["data-footnote-backref"]
          },
          children: Array.isArray(children) ? children : [children]
        });
      }
      const tail = content[content.length - 1];
      if (tail && tail.type === "element" && tail.tagName === "p") {
        const tailTail = tail.children[tail.children.length - 1];
        if (tailTail && tailTail.type === "text") {
          tailTail.value += " ";
        } else {
          tail.children.push({ type: "text", value: " " });
        }
        tail.children.push(...backReferences);
      } else {
        content.push(...backReferences);
      }
      const listItem = {
        type: "element",
        tagName: "li",
        properties: { id: clobberPrefix + "fn-" + safeId },
        children: state.wrap(content, true)
      };
      state.patch(definition, listItem);
      listItems.push(listItem);
    }
    if (listItems.length === 0) {
      return;
    }
    return {
      type: "element",
      tagName: "section",
      properties: { dataFootnotes: true, className: ["footnotes"] },
      children: [
        {
          type: "element",
          tagName: footnoteLabelTagName,
          properties: {
            ...esm_default(footnoteLabelProperties),
            id: "footnote-label"
          },
          children: [{ type: "text", value: footnoteLabel }]
        },
        { type: "text", value: `
` },
        {
          type: "element",
          tagName: "ol",
          properties: {},
          children: state.wrap(listItems, true)
        },
        { type: "text", value: `
` }
      ]
    };
  }

  // ../../node_modules/.bun/unist-util-position@5.0.0/node_modules/unist-util-position/lib/index.js
  var pointEnd = point3("end");
  var pointStart = point3("start");
  function point3(type) {
    return point;
    function point(node) {
      const point = node && node.position && node.position[type] || {};
      if (typeof point.line === "number" && point.line > 0 && typeof point.column === "number" && point.column > 0) {
        return {
          line: point.line,
          column: point.column,
          offset: typeof point.offset === "number" && point.offset > -1 ? point.offset : undefined
        };
      }
    }
  }
  function position2(node) {
    const start = pointStart(node);
    const end = pointEnd(node);
    if (start && end) {
      return { start, end };
    }
  }
  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/blockquote.js
  function blockquote2(state, node) {
    const result = {
      type: "element",
      tagName: "blockquote",
      properties: {},
      children: state.wrap(state.all(node), true)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/break.js
  function hardBreak2(state, node) {
    const result = { type: "element", tagName: "br", properties: {}, children: [] };
    state.patch(node, result);
    return [state.applyData(node, result), { type: "text", value: `
` }];
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/code.js
  function code3(state, node) {
    const value = node.value ? node.value + `
` : "";
    const properties = {};
    const language = node.lang ? node.lang.split(/\s+/) : [];
    if (language.length > 0) {
      properties.className = ["language-" + language[0]];
    }
    let result = {
      type: "element",
      tagName: "code",
      properties,
      children: [{ type: "text", value }]
    };
    if (node.meta) {
      result.data = { meta: node.meta };
    }
    state.patch(node, result);
    result = state.applyData(node, result);
    result = { type: "element", tagName: "pre", properties: {}, children: [result] };
    state.patch(node, result);
    return result;
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/delete.js
  function strikethrough(state, node) {
    const result = {
      type: "element",
      tagName: "del",
      properties: {},
      children: state.all(node)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/emphasis.js
  function emphasis2(state, node) {
    const result = {
      type: "element",
      tagName: "em",
      properties: {},
      children: state.all(node)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/footnote-reference.js
  function footnoteReference2(state, node) {
    const clobberPrefix = typeof state.options.clobberPrefix === "string" ? state.options.clobberPrefix : "user-content-";
    const id = String(node.identifier).toUpperCase();
    const safeId = normalizeUri(id.toLowerCase());
    const index = state.footnoteOrder.indexOf(id);
    let counter;
    let reuseCounter = state.footnoteCounts.get(id);
    if (reuseCounter === undefined) {
      reuseCounter = 0;
      state.footnoteOrder.push(id);
      counter = state.footnoteOrder.length;
    } else {
      counter = index + 1;
    }
    reuseCounter += 1;
    state.footnoteCounts.set(id, reuseCounter);
    const link = {
      type: "element",
      tagName: "a",
      properties: {
        href: "#" + clobberPrefix + "fn-" + safeId,
        id: clobberPrefix + "fnref-" + safeId + (reuseCounter > 1 ? "-" + reuseCounter : ""),
        dataFootnoteRef: true,
        ariaDescribedBy: ["footnote-label"]
      },
      children: [{ type: "text", value: String(counter) }]
    };
    state.patch(node, link);
    const sup = {
      type: "element",
      tagName: "sup",
      properties: {},
      children: [link]
    };
    state.patch(node, sup);
    return state.applyData(node, sup);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/heading.js
  function heading2(state, node) {
    const result = {
      type: "element",
      tagName: "h" + node.depth,
      properties: {},
      children: state.all(node)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/html.js
  function html2(state, node) {
    if (state.options.allowDangerousHtml) {
      const result = { type: "raw", value: node.value };
      state.patch(node, result);
      return state.applyData(node, result);
    }
    return;
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/revert.js
  function revert(state, node) {
    const subtype = node.referenceType;
    let suffix = "]";
    if (subtype === "collapsed") {
      suffix += "[]";
    } else if (subtype === "full") {
      suffix += "[" + (node.label || node.identifier) + "]";
    }
    if (node.type === "imageReference") {
      return [{ type: "text", value: "![" + node.alt + suffix }];
    }
    const contents = state.all(node);
    const head = contents[0];
    if (head && head.type === "text") {
      head.value = "[" + head.value;
    } else {
      contents.unshift({ type: "text", value: "[" });
    }
    const tail = contents[contents.length - 1];
    if (tail && tail.type === "text") {
      tail.value += suffix;
    } else {
      contents.push({ type: "text", value: suffix });
    }
    return contents;
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/image-reference.js
  function imageReference2(state, node) {
    const id = String(node.identifier).toUpperCase();
    const definition = state.definitionById.get(id);
    if (!definition) {
      return revert(state, node);
    }
    const properties = { src: normalizeUri(definition.url || ""), alt: node.alt };
    if (definition.title !== null && definition.title !== undefined) {
      properties.title = definition.title;
    }
    const result = { type: "element", tagName: "img", properties, children: [] };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/image.js
  function image2(state, node) {
    const properties = { src: normalizeUri(node.url) };
    if (node.alt !== null && node.alt !== undefined) {
      properties.alt = node.alt;
    }
    if (node.title !== null && node.title !== undefined) {
      properties.title = node.title;
    }
    const result = { type: "element", tagName: "img", properties, children: [] };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/inline-code.js
  function inlineCode2(state, node) {
    const text = { type: "text", value: node.value.replace(/\r?\n|\r/g, " ") };
    state.patch(node, text);
    const result = {
      type: "element",
      tagName: "code",
      properties: {},
      children: [text]
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/link-reference.js
  function linkReference2(state, node) {
    const id = String(node.identifier).toUpperCase();
    const definition = state.definitionById.get(id);
    if (!definition) {
      return revert(state, node);
    }
    const properties = { href: normalizeUri(definition.url || "") };
    if (definition.title !== null && definition.title !== undefined) {
      properties.title = definition.title;
    }
    const result = {
      type: "element",
      tagName: "a",
      properties,
      children: state.all(node)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/link.js
  function link2(state, node) {
    const properties = { href: normalizeUri(node.url) };
    if (node.title !== null && node.title !== undefined) {
      properties.title = node.title;
    }
    const result = {
      type: "element",
      tagName: "a",
      properties,
      children: state.all(node)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/list-item.js
  function listItem2(state, node, parent) {
    const results = state.all(node);
    const loose = parent ? listLoose(parent) : listItemLoose(node);
    const properties = {};
    const children = [];
    if (typeof node.checked === "boolean") {
      const head = results[0];
      let paragraph;
      if (head && head.type === "element" && head.tagName === "p") {
        paragraph = head;
      } else {
        paragraph = { type: "element", tagName: "p", properties: {}, children: [] };
        results.unshift(paragraph);
      }
      if (paragraph.children.length > 0) {
        paragraph.children.unshift({ type: "text", value: " " });
      }
      paragraph.children.unshift({
        type: "element",
        tagName: "input",
        properties: { type: "checkbox", checked: node.checked, disabled: true },
        children: []
      });
      properties.className = ["task-list-item"];
    }
    let index = -1;
    while (++index < results.length) {
      const child = results[index];
      if (loose || index !== 0 || child.type !== "element" || child.tagName !== "p") {
        children.push({ type: "text", value: `
` });
      }
      if (child.type === "element" && child.tagName === "p" && !loose) {
        children.push(...child.children);
      } else {
        children.push(child);
      }
    }
    const tail = results[results.length - 1];
    if (tail && (loose || tail.type !== "element" || tail.tagName !== "p")) {
      children.push({ type: "text", value: `
` });
    }
    const result = { type: "element", tagName: "li", properties, children };
    state.patch(node, result);
    return state.applyData(node, result);
  }
  function listLoose(node) {
    let loose = false;
    if (node.type === "list") {
      loose = node.spread || false;
      const children = node.children;
      let index = -1;
      while (!loose && ++index < children.length) {
        loose = listItemLoose(children[index]);
      }
    }
    return loose;
  }
  function listItemLoose(node) {
    const spread = node.spread;
    return spread === null || spread === undefined ? node.children.length > 1 : spread;
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/list.js
  function list3(state, node) {
    const properties = {};
    const results = state.all(node);
    let index = -1;
    if (typeof node.start === "number" && node.start !== 1) {
      properties.start = node.start;
    }
    while (++index < results.length) {
      const child = results[index];
      if (child.type === "element" && child.tagName === "li" && child.properties && Array.isArray(child.properties.className) && child.properties.className.includes("task-list-item")) {
        properties.className = ["contains-task-list"];
        break;
      }
    }
    const result = {
      type: "element",
      tagName: node.ordered ? "ol" : "ul",
      properties,
      children: state.wrap(results, true)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/paragraph.js
  function paragraph2(state, node) {
    const result = {
      type: "element",
      tagName: "p",
      properties: {},
      children: state.all(node)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/root.js
  function root2(state, node) {
    const result = { type: "root", children: state.wrap(state.all(node)) };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/strong.js
  function strong2(state, node) {
    const result = {
      type: "element",
      tagName: "strong",
      properties: {},
      children: state.all(node)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/table.js
  function table(state, node) {
    const rows = state.all(node);
    const firstRow = rows.shift();
    const tableContent = [];
    if (firstRow) {
      const head = {
        type: "element",
        tagName: "thead",
        properties: {},
        children: state.wrap([firstRow], true)
      };
      state.patch(node.children[0], head);
      tableContent.push(head);
    }
    if (rows.length > 0) {
      const body = {
        type: "element",
        tagName: "tbody",
        properties: {},
        children: state.wrap(rows, true)
      };
      const start = pointStart(node.children[1]);
      const end = pointEnd(node.children[node.children.length - 1]);
      if (start && end)
        body.position = { start, end };
      tableContent.push(body);
    }
    const result = {
      type: "element",
      tagName: "table",
      properties: {},
      children: state.wrap(tableContent, true)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/table-row.js
  function tableRow(state, node, parent) {
    const siblings = parent ? parent.children : undefined;
    const rowIndex = siblings ? siblings.indexOf(node) : 1;
    const tagName = rowIndex === 0 ? "th" : "td";
    const align = parent && parent.type === "table" ? parent.align : undefined;
    const length = align ? align.length : node.children.length;
    let cellIndex = -1;
    const cells = [];
    while (++cellIndex < length) {
      const cell = node.children[cellIndex];
      const properties = {};
      const alignValue = align ? align[cellIndex] : undefined;
      if (alignValue) {
        properties.align = alignValue;
      }
      let result = { type: "element", tagName, properties, children: [] };
      if (cell) {
        result.children = state.all(cell);
        state.patch(cell, result);
        result = state.applyData(cell, result);
      }
      cells.push(result);
    }
    const result = {
      type: "element",
      tagName: "tr",
      properties: {},
      children: state.wrap(cells, true)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/table-cell.js
  function tableCell(state, node) {
    const result = {
      type: "element",
      tagName: "td",
      properties: {},
      children: state.all(node)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/trim-lines@3.0.1/node_modules/trim-lines/index.js
  var tab = 9;
  var space = 32;
  function trimLines(value) {
    const source = String(value);
    const search = /\r?\n|\r/g;
    let match = search.exec(source);
    let last = 0;
    const lines = [];
    while (match) {
      lines.push(trimLine(source.slice(last, match.index), last > 0, true), match[0]);
      last = match.index + match[0].length;
      match = search.exec(source);
    }
    lines.push(trimLine(source.slice(last), last > 0, false));
    return lines.join("");
  }
  function trimLine(value, start, end) {
    let startIndex = 0;
    let endIndex = value.length;
    if (start) {
      let code = value.codePointAt(startIndex);
      while (code === tab || code === space) {
        startIndex++;
        code = value.codePointAt(startIndex);
      }
    }
    if (end) {
      let code = value.codePointAt(endIndex - 1);
      while (code === tab || code === space) {
        endIndex--;
        code = value.codePointAt(endIndex - 1);
      }
    }
    return endIndex > startIndex ? value.slice(startIndex, endIndex) : "";
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/text.js
  function text5(state, node) {
    const result = { type: "text", value: trimLines(String(node.value)) };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/thematic-break.js
  function thematicBreak3(state, node) {
    const result = {
      type: "element",
      tagName: "hr",
      properties: {},
      children: []
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/handlers/index.js
  var handlers = {
    blockquote: blockquote2,
    break: hardBreak2,
    code: code3,
    delete: strikethrough,
    emphasis: emphasis2,
    footnoteReference: footnoteReference2,
    heading: heading2,
    html: html2,
    imageReference: imageReference2,
    image: image2,
    inlineCode: inlineCode2,
    linkReference: linkReference2,
    link: link2,
    listItem: listItem2,
    list: list3,
    paragraph: paragraph2,
    root: root2,
    strong: strong2,
    table,
    tableCell,
    tableRow,
    text: text5,
    thematicBreak: thematicBreak3,
    toml: ignore,
    yaml: ignore,
    definition: ignore,
    footnoteDefinition: ignore
  };
  function ignore() {
    return;
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/state.js
  var own2 = {}.hasOwnProperty;
  var emptyOptions3 = {};
  function createState(tree, options) {
    const settings = options || emptyOptions3;
    const definitionById = new Map;
    const footnoteById = new Map;
    const footnoteCounts = new Map;
    const handlers2 = { ...handlers, ...settings.handlers };
    const state = {
      all,
      applyData,
      definitionById,
      footnoteById,
      footnoteCounts,
      footnoteOrder: [],
      handlers: handlers2,
      one,
      options: settings,
      patch,
      wrap
    };
    visit(tree, function(node) {
      if (node.type === "definition" || node.type === "footnoteDefinition") {
        const map = node.type === "definition" ? definitionById : footnoteById;
        const id = String(node.identifier).toUpperCase();
        if (!map.has(id)) {
          map.set(id, node);
        }
      }
    });
    return state;
    function one(node, parent) {
      const type = node.type;
      const handle = state.handlers[type];
      if (own2.call(state.handlers, type) && handle) {
        return handle(state, node, parent);
      }
      if (state.options.passThrough && state.options.passThrough.includes(type)) {
        if ("children" in node) {
          const { children, ...shallow } = node;
          const result = esm_default(shallow);
          result.children = state.all(node);
          return result;
        }
        return esm_default(node);
      }
      const unknown = state.options.unknownHandler || defaultUnknownHandler;
      return unknown(state, node, parent);
    }
    function all(parent) {
      const values = [];
      if ("children" in parent) {
        const nodes = parent.children;
        let index = -1;
        while (++index < nodes.length) {
          const result = state.one(nodes[index], parent);
          if (result) {
            if (index && nodes[index - 1].type === "break") {
              if (!Array.isArray(result) && result.type === "text") {
                result.value = trimMarkdownSpaceStart(result.value);
              }
              if (!Array.isArray(result) && result.type === "element") {
                const head = result.children[0];
                if (head && head.type === "text") {
                  head.value = trimMarkdownSpaceStart(head.value);
                }
              }
            }
            if (Array.isArray(result)) {
              values.push(...result);
            } else {
              values.push(result);
            }
          }
        }
      }
      return values;
    }
  }
  function patch(from, to) {
    if (from.position)
      to.position = position2(from);
  }
  function applyData(from, to) {
    let result = to;
    if (from && from.data) {
      const hName = from.data.hName;
      const hChildren = from.data.hChildren;
      const hProperties = from.data.hProperties;
      if (typeof hName === "string") {
        if (result.type === "element") {
          result.tagName = hName;
        } else {
          const children = "children" in result ? result.children : [result];
          result = { type: "element", tagName: hName, properties: {}, children };
        }
      }
      if (result.type === "element" && hProperties) {
        Object.assign(result.properties, esm_default(hProperties));
      }
      if ("children" in result && result.children && hChildren !== null && hChildren !== undefined) {
        result.children = hChildren;
      }
    }
    return result;
  }
  function defaultUnknownHandler(state, node) {
    const data = node.data || {};
    const result = "value" in node && !(own2.call(data, "hProperties") || own2.call(data, "hChildren")) ? { type: "text", value: node.value } : {
      type: "element",
      tagName: "div",
      properties: {},
      children: state.all(node)
    };
    state.patch(node, result);
    return state.applyData(node, result);
  }
  function wrap(nodes, loose) {
    const result = [];
    let index = -1;
    if (loose) {
      result.push({ type: "text", value: `
` });
    }
    while (++index < nodes.length) {
      if (index)
        result.push({ type: "text", value: `
` });
      result.push(nodes[index]);
    }
    if (loose && nodes.length > 0) {
      result.push({ type: "text", value: `
` });
    }
    return result;
  }
  function trimMarkdownSpaceStart(value) {
    let index = 0;
    let code = value.charCodeAt(index);
    while (code === 9 || code === 32) {
      index++;
      code = value.charCodeAt(index);
    }
    return value.slice(index);
  }

  // ../../node_modules/.bun/mdast-util-to-hast@13.2.1/node_modules/mdast-util-to-hast/lib/index.js
  function toHast(tree, options) {
    const state = createState(tree, options);
    const node = state.one(tree, undefined);
    const foot = footer(state);
    const result = Array.isArray(node) ? { type: "root", children: node } : node || { type: "root", children: [] };
    if (foot) {
      ok("children" in result);
      result.children.push({ type: "text", value: `
` }, foot);
    }
    return result;
  }
  // ../../node_modules/.bun/remark-rehype@11.1.2/node_modules/remark-rehype/lib/index.js
  function remarkRehype(destination, options) {
    if (destination && "run" in destination) {
      return async function(tree, file) {
        const hastTree = toHast(tree, { file, ...options });
        await destination.run(hastTree, file);
      };
    }
    return function(tree, file) {
      return toHast(tree, { file, ...destination || options });
    };
  }
  // ../../node_modules/.bun/bail@2.0.2/node_modules/bail/index.js
  function bail(error) {
    if (error) {
      throw error;
    }
  }

  // ../../node_modules/.bun/unified@11.0.5/node_modules/unified/lib/index.js
  var import_extend = __toESM(require_extend(), 1);

  // ../../node_modules/.bun/is-plain-obj@4.1.0/node_modules/is-plain-obj/index.js
  function isPlainObject(value) {
    if (typeof value !== "object" || value === null) {
      return false;
    }
    const prototype = Object.getPrototypeOf(value);
    return (prototype === null || prototype === Object.prototype || Object.getPrototypeOf(prototype) === null) && !(Symbol.toStringTag in value) && !(Symbol.iterator in value);
  }

  // ../../node_modules/.bun/trough@2.2.0/node_modules/trough/lib/index.js
  function trough() {
    const fns = [];
    const pipeline = { run, use };
    return pipeline;
    function run(...values) {
      let middlewareIndex = -1;
      const callback = values.pop();
      if (typeof callback !== "function") {
        throw new TypeError("Expected function as last argument, not " + callback);
      }
      next(null, ...values);
      function next(error, ...output) {
        const fn = fns[++middlewareIndex];
        let index = -1;
        if (error) {
          callback(error);
          return;
        }
        while (++index < values.length) {
          if (output[index] === null || output[index] === undefined) {
            output[index] = values[index];
          }
        }
        values = output;
        if (fn) {
          wrap2(fn, next)(...output);
        } else {
          callback(null, ...output);
        }
      }
    }
    function use(middelware) {
      if (typeof middelware !== "function") {
        throw new TypeError("Expected `middelware` to be a function, not " + middelware);
      }
      fns.push(middelware);
      return pipeline;
    }
  }
  function wrap2(middleware, callback) {
    let called;
    return wrapped;
    function wrapped(...parameters) {
      const fnExpectsCallback = middleware.length > parameters.length;
      let result;
      if (fnExpectsCallback) {
        parameters.push(done);
      }
      try {
        result = middleware.apply(this, parameters);
      } catch (error) {
        const exception = error;
        if (fnExpectsCallback && called) {
          throw exception;
        }
        return done(exception);
      }
      if (!fnExpectsCallback) {
        if (result && result.then && typeof result.then === "function") {
          result.then(then, done);
        } else if (result instanceof Error) {
          done(result);
        } else {
          then(result);
        }
      }
    }
    function done(error, ...output) {
      if (!called) {
        called = true;
        callback(error, ...output);
      }
    }
    function then(value) {
      done(null, value);
    }
  }
  // ../../node_modules/.bun/vfile-message@4.0.3/node_modules/vfile-message/lib/index.js
  class VFileMessage extends Error {
    constructor(causeOrReason, optionsOrParentOrPlace, origin) {
      super();
      if (typeof optionsOrParentOrPlace === "string") {
        origin = optionsOrParentOrPlace;
        optionsOrParentOrPlace = undefined;
      }
      let reason = "";
      let options = {};
      let legacyCause = false;
      if (optionsOrParentOrPlace) {
        if ("line" in optionsOrParentOrPlace && "column" in optionsOrParentOrPlace) {
          options = { place: optionsOrParentOrPlace };
        } else if ("start" in optionsOrParentOrPlace && "end" in optionsOrParentOrPlace) {
          options = { place: optionsOrParentOrPlace };
        } else if ("type" in optionsOrParentOrPlace) {
          options = {
            ancestors: [optionsOrParentOrPlace],
            place: optionsOrParentOrPlace.position
          };
        } else {
          options = { ...optionsOrParentOrPlace };
        }
      }
      if (typeof causeOrReason === "string") {
        reason = causeOrReason;
      } else if (!options.cause && causeOrReason) {
        legacyCause = true;
        reason = causeOrReason.message;
        options.cause = causeOrReason;
      }
      if (!options.ruleId && !options.source && typeof origin === "string") {
        const index = origin.indexOf(":");
        if (index === -1) {
          options.ruleId = origin;
        } else {
          options.source = origin.slice(0, index);
          options.ruleId = origin.slice(index + 1);
        }
      }
      if (!options.place && options.ancestors && options.ancestors) {
        const parent = options.ancestors[options.ancestors.length - 1];
        if (parent) {
          options.place = parent.position;
        }
      }
      const start = options.place && "start" in options.place ? options.place.start : options.place;
      this.ancestors = options.ancestors || undefined;
      this.cause = options.cause || undefined;
      this.column = start ? start.column : undefined;
      this.fatal = undefined;
      this.file = "";
      this.message = reason;
      this.line = start ? start.line : undefined;
      this.name = stringifyPosition(options.place) || "1:1";
      this.place = options.place || undefined;
      this.reason = this.message;
      this.ruleId = options.ruleId || undefined;
      this.source = options.source || undefined;
      this.stack = legacyCause && options.cause && typeof options.cause.stack === "string" ? options.cause.stack : "";
      this.actual = undefined;
      this.expected = undefined;
      this.note = undefined;
      this.url = undefined;
    }
  }
  VFileMessage.prototype.file = "";
  VFileMessage.prototype.name = "";
  VFileMessage.prototype.reason = "";
  VFileMessage.prototype.message = "";
  VFileMessage.prototype.stack = "";
  VFileMessage.prototype.column = undefined;
  VFileMessage.prototype.line = undefined;
  VFileMessage.prototype.ancestors = undefined;
  VFileMessage.prototype.cause = undefined;
  VFileMessage.prototype.fatal = undefined;
  VFileMessage.prototype.place = undefined;
  VFileMessage.prototype.ruleId = undefined;
  VFileMessage.prototype.source = undefined;
  // ../../node_modules/.bun/vfile@6.0.3/node_modules/vfile/lib/minpath.browser.js
  var minpath = { basename, dirname, extname, join, sep: "/" };
  function basename(path, extname) {
    if (extname !== undefined && typeof extname !== "string") {
      throw new TypeError('"ext" argument must be a string');
    }
    assertPath(path);
    let start = 0;
    let end = -1;
    let index = path.length;
    let seenNonSlash;
    if (extname === undefined || extname.length === 0 || extname.length > path.length) {
      while (index--) {
        if (path.codePointAt(index) === 47) {
          if (seenNonSlash) {
            start = index + 1;
            break;
          }
        } else if (end < 0) {
          seenNonSlash = true;
          end = index + 1;
        }
      }
      return end < 0 ? "" : path.slice(start, end);
    }
    if (extname === path) {
      return "";
    }
    let firstNonSlashEnd = -1;
    let extnameIndex = extname.length - 1;
    while (index--) {
      if (path.codePointAt(index) === 47) {
        if (seenNonSlash) {
          start = index + 1;
          break;
        }
      } else {
        if (firstNonSlashEnd < 0) {
          seenNonSlash = true;
          firstNonSlashEnd = index + 1;
        }
        if (extnameIndex > -1) {
          if (path.codePointAt(index) === extname.codePointAt(extnameIndex--)) {
            if (extnameIndex < 0) {
              end = index;
            }
          } else {
            extnameIndex = -1;
            end = firstNonSlashEnd;
          }
        }
      }
    }
    if (start === end) {
      end = firstNonSlashEnd;
    } else if (end < 0) {
      end = path.length;
    }
    return path.slice(start, end);
  }
  function dirname(path) {
    assertPath(path);
    if (path.length === 0) {
      return ".";
    }
    let end = -1;
    let index = path.length;
    let unmatchedSlash;
    while (--index) {
      if (path.codePointAt(index) === 47) {
        if (unmatchedSlash) {
          end = index;
          break;
        }
      } else if (!unmatchedSlash) {
        unmatchedSlash = true;
      }
    }
    return end < 0 ? path.codePointAt(0) === 47 ? "/" : "." : end === 1 && path.codePointAt(0) === 47 ? "//" : path.slice(0, end);
  }
  function extname(path) {
    assertPath(path);
    let index = path.length;
    let end = -1;
    let startPart = 0;
    let startDot = -1;
    let preDotState = 0;
    let unmatchedSlash;
    while (index--) {
      const code = path.codePointAt(index);
      if (code === 47) {
        if (unmatchedSlash) {
          startPart = index + 1;
          break;
        }
        continue;
      }
      if (end < 0) {
        unmatchedSlash = true;
        end = index + 1;
      }
      if (code === 46) {
        if (startDot < 0) {
          startDot = index;
        } else if (preDotState !== 1) {
          preDotState = 1;
        }
      } else if (startDot > -1) {
        preDotState = -1;
      }
    }
    if (startDot < 0 || end < 0 || preDotState === 0 || preDotState === 1 && startDot === end - 1 && startDot === startPart + 1) {
      return "";
    }
    return path.slice(startDot, end);
  }
  function join(...segments) {
    let index = -1;
    let joined;
    while (++index < segments.length) {
      assertPath(segments[index]);
      if (segments[index]) {
        joined = joined === undefined ? segments[index] : joined + "/" + segments[index];
      }
    }
    return joined === undefined ? "." : normalize(joined);
  }
  function normalize(path) {
    assertPath(path);
    const absolute = path.codePointAt(0) === 47;
    let value = normalizeString(path, !absolute);
    if (value.length === 0 && !absolute) {
      value = ".";
    }
    if (value.length > 0 && path.codePointAt(path.length - 1) === 47) {
      value += "/";
    }
    return absolute ? "/" + value : value;
  }
  function normalizeString(path, allowAboveRoot) {
    let result = "";
    let lastSegmentLength = 0;
    let lastSlash = -1;
    let dots = 0;
    let index = -1;
    let code;
    let lastSlashIndex;
    while (++index <= path.length) {
      if (index < path.length) {
        code = path.codePointAt(index);
      } else if (code === 47) {
        break;
      } else {
        code = 47;
      }
      if (code === 47) {
        if (lastSlash === index - 1 || dots === 1) {} else if (lastSlash !== index - 1 && dots === 2) {
          if (result.length < 2 || lastSegmentLength !== 2 || result.codePointAt(result.length - 1) !== 46 || result.codePointAt(result.length - 2) !== 46) {
            if (result.length > 2) {
              lastSlashIndex = result.lastIndexOf("/");
              if (lastSlashIndex !== result.length - 1) {
                if (lastSlashIndex < 0) {
                  result = "";
                  lastSegmentLength = 0;
                } else {
                  result = result.slice(0, lastSlashIndex);
                  lastSegmentLength = result.length - 1 - result.lastIndexOf("/");
                }
                lastSlash = index;
                dots = 0;
                continue;
              }
            } else if (result.length > 0) {
              result = "";
              lastSegmentLength = 0;
              lastSlash = index;
              dots = 0;
              continue;
            }
          }
          if (allowAboveRoot) {
            result = result.length > 0 ? result + "/.." : "..";
            lastSegmentLength = 2;
          }
        } else {
          if (result.length > 0) {
            result += "/" + path.slice(lastSlash + 1, index);
          } else {
            result = path.slice(lastSlash + 1, index);
          }
          lastSegmentLength = index - lastSlash - 1;
        }
        lastSlash = index;
        dots = 0;
      } else if (code === 46 && dots > -1) {
        dots++;
      } else {
        dots = -1;
      }
    }
    return result;
  }
  function assertPath(path) {
    if (typeof path !== "string") {
      throw new TypeError("Path must be a string. Received " + JSON.stringify(path));
    }
  }

  // ../../node_modules/.bun/vfile@6.0.3/node_modules/vfile/lib/minproc.browser.js
  var minproc = { cwd };
  function cwd() {
    return "/";
  }

  // ../../node_modules/.bun/vfile@6.0.3/node_modules/vfile/lib/minurl.shared.js
  function isUrl(fileUrlOrPath) {
    return Boolean(fileUrlOrPath !== null && typeof fileUrlOrPath === "object" && "href" in fileUrlOrPath && fileUrlOrPath.href && "protocol" in fileUrlOrPath && fileUrlOrPath.protocol && fileUrlOrPath.auth === undefined);
  }

  // ../../node_modules/.bun/vfile@6.0.3/node_modules/vfile/lib/minurl.browser.js
  function urlToPath(path) {
    if (typeof path === "string") {
      path = new URL(path);
    } else if (!isUrl(path)) {
      const error = new TypeError('The "path" argument must be of type string or an instance of URL. Received `' + path + "`");
      error.code = "ERR_INVALID_ARG_TYPE";
      throw error;
    }
    if (path.protocol !== "file:") {
      const error = new TypeError("The URL must be of scheme file");
      error.code = "ERR_INVALID_URL_SCHEME";
      throw error;
    }
    return getPathFromURLPosix(path);
  }
  function getPathFromURLPosix(url) {
    if (url.hostname !== "") {
      const error = new TypeError('File URL host must be "localhost" or empty on darwin');
      error.code = "ERR_INVALID_FILE_URL_HOST";
      throw error;
    }
    const pathname = url.pathname;
    let index = -1;
    while (++index < pathname.length) {
      if (pathname.codePointAt(index) === 37 && pathname.codePointAt(index + 1) === 50) {
        const third = pathname.codePointAt(index + 2);
        if (third === 70 || third === 102) {
          const error = new TypeError("File URL path must not include encoded / characters");
          error.code = "ERR_INVALID_FILE_URL_PATH";
          throw error;
        }
      }
    }
    return decodeURIComponent(pathname);
  }

  // ../../node_modules/.bun/vfile@6.0.3/node_modules/vfile/lib/index.js
  var order = [
    "history",
    "path",
    "basename",
    "stem",
    "extname",
    "dirname"
  ];

  class VFile {
    constructor(value) {
      let options;
      if (!value) {
        options = {};
      } else if (isUrl(value)) {
        options = { path: value };
      } else if (typeof value === "string" || isUint8Array(value)) {
        options = { value };
      } else {
        options = value;
      }
      this.cwd = "cwd" in options ? "" : minproc.cwd();
      this.data = {};
      this.history = [];
      this.messages = [];
      this.value;
      this.map;
      this.result;
      this.stored;
      let index = -1;
      while (++index < order.length) {
        const field = order[index];
        if (field in options && options[field] !== undefined && options[field] !== null) {
          this[field] = field === "history" ? [...options[field]] : options[field];
        }
      }
      let field;
      for (field in options) {
        if (!order.includes(field)) {
          this[field] = options[field];
        }
      }
    }
    get basename() {
      return typeof this.path === "string" ? minpath.basename(this.path) : undefined;
    }
    set basename(basename) {
      assertNonEmpty(basename, "basename");
      assertPart(basename, "basename");
      this.path = minpath.join(this.dirname || "", basename);
    }
    get dirname() {
      return typeof this.path === "string" ? minpath.dirname(this.path) : undefined;
    }
    set dirname(dirname) {
      assertPath2(this.basename, "dirname");
      this.path = minpath.join(dirname || "", this.basename);
    }
    get extname() {
      return typeof this.path === "string" ? minpath.extname(this.path) : undefined;
    }
    set extname(extname) {
      assertPart(extname, "extname");
      assertPath2(this.dirname, "extname");
      if (extname) {
        if (extname.codePointAt(0) !== 46) {
          throw new Error("`extname` must start with `.`");
        }
        if (extname.includes(".", 1)) {
          throw new Error("`extname` cannot contain multiple dots");
        }
      }
      this.path = minpath.join(this.dirname, this.stem + (extname || ""));
    }
    get path() {
      return this.history[this.history.length - 1];
    }
    set path(path) {
      if (isUrl(path)) {
        path = urlToPath(path);
      }
      assertNonEmpty(path, "path");
      if (this.path !== path) {
        this.history.push(path);
      }
    }
    get stem() {
      return typeof this.path === "string" ? minpath.basename(this.path, this.extname) : undefined;
    }
    set stem(stem) {
      assertNonEmpty(stem, "stem");
      assertPart(stem, "stem");
      this.path = minpath.join(this.dirname || "", stem + (this.extname || ""));
    }
    fail(causeOrReason, optionsOrParentOrPlace, origin) {
      const message = this.message(causeOrReason, optionsOrParentOrPlace, origin);
      message.fatal = true;
      throw message;
    }
    info(causeOrReason, optionsOrParentOrPlace, origin) {
      const message = this.message(causeOrReason, optionsOrParentOrPlace, origin);
      message.fatal = undefined;
      return message;
    }
    message(causeOrReason, optionsOrParentOrPlace, origin) {
      const message = new VFileMessage(causeOrReason, optionsOrParentOrPlace, origin);
      if (this.path) {
        message.name = this.path + ":" + message.name;
        message.file = this.path;
      }
      message.fatal = false;
      this.messages.push(message);
      return message;
    }
    toString(encoding) {
      if (this.value === undefined) {
        return "";
      }
      if (typeof this.value === "string") {
        return this.value;
      }
      const decoder = new TextDecoder(encoding || undefined);
      return decoder.decode(this.value);
    }
  }
  function assertPart(part, name) {
    if (part && part.includes(minpath.sep)) {
      throw new Error("`" + name + "` cannot be a path: did not expect `" + minpath.sep + "`");
    }
  }
  function assertNonEmpty(part, name) {
    if (!part) {
      throw new Error("`" + name + "` cannot be empty");
    }
  }
  function assertPath2(path, name) {
    if (!path) {
      throw new Error("Setting `" + name + "` requires `path` to be set too");
    }
  }
  function isUint8Array(value) {
    return Boolean(value && typeof value === "object" && "byteLength" in value && "byteOffset" in value);
  }
  // ../../node_modules/.bun/unified@11.0.5/node_modules/unified/lib/callable-instance.js
  var CallableInstance = function(property) {
    const self2 = this;
    const constr = self2.constructor;
    const proto = constr.prototype;
    const value = proto[property];
    const apply = function() {
      return value.apply(apply, arguments);
    };
    Object.setPrototypeOf(apply, proto);
    return apply;
  };

  // ../../node_modules/.bun/unified@11.0.5/node_modules/unified/lib/index.js
  var own3 = {}.hasOwnProperty;

  class Processor extends CallableInstance {
    constructor() {
      super("copy");
      this.Compiler = undefined;
      this.Parser = undefined;
      this.attachers = [];
      this.compiler = undefined;
      this.freezeIndex = -1;
      this.frozen = undefined;
      this.namespace = {};
      this.parser = undefined;
      this.transformers = trough();
    }
    copy() {
      const destination = new Processor;
      let index = -1;
      while (++index < this.attachers.length) {
        const attacher = this.attachers[index];
        destination.use(...attacher);
      }
      destination.data(import_extend.default(true, {}, this.namespace));
      return destination;
    }
    data(key, value) {
      if (typeof key === "string") {
        if (arguments.length === 2) {
          assertUnfrozen("data", this.frozen);
          this.namespace[key] = value;
          return this;
        }
        return own3.call(this.namespace, key) && this.namespace[key] || undefined;
      }
      if (key) {
        assertUnfrozen("data", this.frozen);
        this.namespace = key;
        return this;
      }
      return this.namespace;
    }
    freeze() {
      if (this.frozen) {
        return this;
      }
      const self2 = this;
      while (++this.freezeIndex < this.attachers.length) {
        const [attacher, ...options] = this.attachers[this.freezeIndex];
        if (options[0] === false) {
          continue;
        }
        if (options[0] === true) {
          options[0] = undefined;
        }
        const transformer = attacher.call(self2, ...options);
        if (typeof transformer === "function") {
          this.transformers.use(transformer);
        }
      }
      this.frozen = true;
      this.freezeIndex = Number.POSITIVE_INFINITY;
      return this;
    }
    parse(file) {
      this.freeze();
      const realFile = vfile(file);
      const parser = this.parser || this.Parser;
      assertParser("parse", parser);
      return parser(String(realFile), realFile);
    }
    process(file, done) {
      const self2 = this;
      this.freeze();
      assertParser("process", this.parser || this.Parser);
      assertCompiler("process", this.compiler || this.Compiler);
      return done ? executor(undefined, done) : new Promise(executor);
      function executor(resolve, reject) {
        const realFile = vfile(file);
        const parseTree = self2.parse(realFile);
        self2.run(parseTree, realFile, function(error, tree, file) {
          if (error || !tree || !file) {
            return realDone(error);
          }
          const compileTree = tree;
          const compileResult = self2.stringify(compileTree, file);
          if (looksLikeAValue(compileResult)) {
            file.value = compileResult;
          } else {
            file.result = compileResult;
          }
          realDone(error, file);
        });
        function realDone(error, file) {
          if (error || !file) {
            reject(error);
          } else if (resolve) {
            resolve(file);
          } else {
            ok(done, "`done` is defined if `resolve` is not");
            done(undefined, file);
          }
        }
      }
    }
    processSync(file) {
      let complete = false;
      let result;
      this.freeze();
      assertParser("processSync", this.parser || this.Parser);
      assertCompiler("processSync", this.compiler || this.Compiler);
      this.process(file, realDone);
      assertDone("processSync", "process", complete);
      ok(result, "we either bailed on an error or have a tree");
      return result;
      function realDone(error, file) {
        complete = true;
        bail(error);
        result = file;
      }
    }
    run(tree, file, done) {
      assertNode(tree);
      this.freeze();
      const transformers = this.transformers;
      if (!done && typeof file === "function") {
        done = file;
        file = undefined;
      }
      return done ? executor(undefined, done) : new Promise(executor);
      function executor(resolve, reject) {
        ok(typeof file !== "function", "`file` can’t be a `done` anymore, we checked");
        const realFile = vfile(file);
        transformers.run(tree, realFile, realDone);
        function realDone(error, outputTree, file) {
          const resultingTree = outputTree || tree;
          if (error) {
            reject(error);
          } else if (resolve) {
            resolve(resultingTree);
          } else {
            ok(done, "`done` is defined if `resolve` is not");
            done(undefined, resultingTree, file);
          }
        }
      }
    }
    runSync(tree, file) {
      let complete = false;
      let result;
      this.run(tree, file, realDone);
      assertDone("runSync", "run", complete);
      ok(result, "we either bailed on an error or have a tree");
      return result;
      function realDone(error, tree) {
        bail(error);
        result = tree;
        complete = true;
      }
    }
    stringify(tree, file) {
      this.freeze();
      const realFile = vfile(file);
      const compiler = this.compiler || this.Compiler;
      assertCompiler("stringify", compiler);
      assertNode(tree);
      return compiler(tree, realFile);
    }
    use(value, ...parameters) {
      const attachers = this.attachers;
      const namespace = this.namespace;
      assertUnfrozen("use", this.frozen);
      if (value === null || value === undefined) {} else if (typeof value === "function") {
        addPlugin(value, parameters);
      } else if (typeof value === "object") {
        if (Array.isArray(value)) {
          addList(value);
        } else {
          addPreset(value);
        }
      } else {
        throw new TypeError("Expected usable value, not `" + value + "`");
      }
      return this;
      function add(value) {
        if (typeof value === "function") {
          addPlugin(value, []);
        } else if (typeof value === "object") {
          if (Array.isArray(value)) {
            const [plugin, ...parameters] = value;
            addPlugin(plugin, parameters);
          } else {
            addPreset(value);
          }
        } else {
          throw new TypeError("Expected usable value, not `" + value + "`");
        }
      }
      function addPreset(result) {
        if (!("plugins" in result) && !("settings" in result)) {
          throw new Error("Expected usable value but received an empty preset, which is probably a mistake: presets typically come with `plugins` and sometimes with `settings`, but this has neither");
        }
        addList(result.plugins);
        if (result.settings) {
          namespace.settings = import_extend.default(true, namespace.settings, result.settings);
        }
      }
      function addList(plugins) {
        let index = -1;
        if (plugins === null || plugins === undefined) {} else if (Array.isArray(plugins)) {
          while (++index < plugins.length) {
            const thing = plugins[index];
            add(thing);
          }
        } else {
          throw new TypeError("Expected a list of plugins, not `" + plugins + "`");
        }
      }
      function addPlugin(plugin, parameters) {
        let index = -1;
        let entryIndex = -1;
        while (++index < attachers.length) {
          if (attachers[index][0] === plugin) {
            entryIndex = index;
            break;
          }
        }
        if (entryIndex === -1) {
          attachers.push([plugin, ...parameters]);
        } else if (parameters.length > 0) {
          let [primary, ...rest] = parameters;
          const currentPrimary = attachers[entryIndex][1];
          if (isPlainObject(currentPrimary) && isPlainObject(primary)) {
            primary = import_extend.default(true, currentPrimary, primary);
          }
          attachers[entryIndex] = [plugin, primary, ...rest];
        }
      }
    }
  }
  var unified = new Processor().freeze();
  function assertParser(name, value) {
    if (typeof value !== "function") {
      throw new TypeError("Cannot `" + name + "` without `parser`");
    }
  }
  function assertCompiler(name, value) {
    if (typeof value !== "function") {
      throw new TypeError("Cannot `" + name + "` without `compiler`");
    }
  }
  function assertUnfrozen(name, frozen) {
    if (frozen) {
      throw new Error("Cannot call `" + name + "` on a frozen processor.\nCreate a new processor first, by calling it: use `processor()` instead of `processor`.");
    }
  }
  function assertNode(node) {
    if (!isPlainObject(node) || typeof node.type !== "string") {
      throw new TypeError("Expected node, got `" + node + "`");
    }
  }
  function assertDone(name, asyncName, complete) {
    if (!complete) {
      throw new Error("`" + name + "` finished async. Use `" + asyncName + "` instead");
    }
  }
  function vfile(value) {
    return looksLikeAVFile(value) ? value : new VFile(value);
  }
  function looksLikeAVFile(value) {
    return Boolean(value && typeof value === "object" && "message" in value && "messages" in value);
  }
  function looksLikeAValue(value) {
    return typeof value === "string" || isUint8Array2(value);
  }
  function isUint8Array2(value) {
    return Boolean(value && typeof value === "object" && "byteLength" in value && "byteOffset" in value);
  }
  // src/components/Markdown/inlineMarkdown.ts
  var INLINE_ELEMENTS = ["a", "strong", "em", "del", "code", "br"];

  // src/external-url-allowlist.ts
  var ALLOWED_SCHEMES = new Set(["http:", "https:"]);

  // src/tabs/pageRefs.ts
  function fileRef(path, version = WORKING, filesystem = null) {
    return { id: fileUri(path, revisionSlot(version), filesystem) };
  }
  function directoryRef(path) {
    const bare = path.replace(/\/+$/, "");
    return { id: dirUri(bare) };
  }
  function knowledgePageRef(store, id) {
    return { id: knowledgeUri(store, id) };
  }
  function gitCommitRef(sha) {
    return { id: commitUri(sha) };
  }

  // src/knowledgeStoreIds.ts
  var OXWIKI = "oxwiki";
  var ids = [];
  function enabledKnowledgeStoreIds() {
    return ids;
  }

  // src/taskRef.ts
  function taskId(ref) {
    const item = taskOf(ref);
    return item === null ? null : `${item.provider}:${item.id}`;
  }

  // src/pages/renderers.ts
  var RENDERER_VIEW_KEY = "oxplow.renderer";

  // src/resourceStore.ts
  var current = new Map;
  var listeners = new Set;
  function resourceInfo(resource) {
    return current.get(resource) ?? null;
  }
  function acceptsParam(resource, param, id) {
    const pattern = resource.params[param]?.pattern;
    if (!pattern)
      return true;
    try {
      return new RegExp(pattern).test(id);
    } catch {
      return false;
    }
  }
  function extensionWikilinkRef(interior) {
    const colon = interior.indexOf(":");
    if (colon <= 0)
      return null;
    const head = interior.slice(0, colon).toLowerCase();
    const id = interior.slice(colon + 1).trim();
    if (!id)
      return null;
    const all = [...current.values()];
    const declared = all.find((r) => r.wikilink?.prefix === head);
    if (declared?.wikilink) {
      const { param } = declared.wikilink;
      return acceptsParam(declared, param, id) ? resourceUri(declared.resource, { [param]: id }) : null;
    }
    const named = current.get(`oxplow/${head}`);
    const keys = named ? Object.keys(named.params) : [];
    if (named && keys.length === 1 && named.params[keys[0]].required) {
      return acceptsParam(named, keys[0], id) ? resourceUri(named.resource, { [keys[0]]: id }) : null;
    }
    return null;
  }

  // src/components/Markdown/markdownLinks.ts
  var INVALID_LINK = "oxplow-invalid:";
  var SAFE_PROTOCOL = /^(https?|ircs?|mailto|xmpp)$/i;
  function urlTransform(value) {
    if (isOxplowScheme(value) || value.startsWith(INVALID_LINK))
      return value;
    const colon = value.indexOf(":");
    const before = (c) => {
      const at = value.indexOf(c);
      return at !== -1 && colon > at;
    };
    if (colon === -1 || before("/") || before("?") || before("#") || SAFE_PROTOCOL.test(value.slice(0, colon)))
      return value;
    return "";
  }
  var SHA_RE = /^[0-9a-f]{7,40}$/i;
  function parseGitRefTarget(target) {
    const stripped = target.startsWith("git:") ? target.slice(4) : target;
    if (!SHA_RE.test(stripped))
      return null;
    return stripped.toLowerCase();
  }
  function parseMarkdownLink(rawHref, base = null) {
    if (!rawHref)
      return { kind: "empty" };
    if (rawHref.startsWith("#"))
      return { kind: "anchor" };
    if (/^https?:\/\//i.test(rawHref) || rawHref.startsWith("mailto:")) {
      return { kind: "external" };
    }
    const uri = parseUri(rawHref);
    if (uri !== null) {
      const p = uri.params;
      switch (typeOf(rawHref)) {
        case "oxplow/file": {
          if (!p.path)
            return { kind: "empty" };
          const line = Number(/^(\d+)-\d+$/.exec(uri.view.lines ?? "")?.[1]);
          const version = p.rev ? revisionFromSlot(p.rev) : null;
          return Number.isInteger(line) ? { kind: "file", path: p.path, line, version } : { kind: "file", path: p.path, version };
        }
        case "oxplow/dir": {
          const path = (p.path ?? "").replace(/\/+$/, "");
          return path ? { kind: "directory", path } : { kind: "empty" };
        }
        case "oxplow/commit":
          return p.sha ? { kind: "commit", sha: p.sha } : { kind: "empty" };
        case "oxplow/knowledge":
          return p.store && p.id ? { kind: "knowledge", store: p.store, id: p.id } : { kind: "empty" };
        case "oxplow/task":
          return taskId(rawHref) === null ? { kind: "empty" } : { kind: "task", ref: identityOf(rawHref) ?? rawHref };
        default: {
          const resource = typeOf(rawHref);
          if (resource && resourceInfo(resource))
            return { kind: "ref", ref: rawHref };
        }
      }
    }
    if (rawHref.startsWith(INVALID_LINK)) {
      const target = decodeURIComponent(rawHref.slice(INVALID_LINK.length));
      return {
        kind: "broken",
        reason: `Broken link: “${target}” is not a recognized reference (use a task's id, [[some-slug]] for an oxwiki page, [[store:id]] for another store's)`
      };
    }
    const relativeTo = base !== null && typeOf(base) === "oxplow/file" ? parseUri(base) : null;
    if (relativeTo?.params.path && !/^[a-z][a-z0-9+.-]*:/i.test(rawHref)) {
      const target = rawHref.split("#")[0]?.split("?")[0] ?? "";
      if (!target)
        return { kind: "empty" };
      const p = relativeTo.params;
      const path = relativeFilePath(p.path, safeDecode(target));
      const renderer = relativeTo.view[RENDERER_VIEW_KEY];
      return {
        kind: "file",
        path,
        version: p.rev ? revisionFromSlot(p.rev) : null,
        filesystem: p.filesystem ?? null,
        ...renderer && extensionOf(path) === extensionOf(p.path) ? { renderer } : {}
      };
    }
    let target = rawHref.replace(/^\.?\//, "");
    target = target.split("#")[0]?.split("?")[0] ?? "";
    if (target.endsWith(".md"))
      target = target.slice(0, -3);
    return target ? { kind: "knowledge", store: OXWIKI, id: target } : { kind: "empty" };
  }
  function extensionOf(path) {
    const name = path.split("/").pop() ?? "";
    const dot = name.lastIndexOf(".");
    return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  }
  function safeDecode(text) {
    try {
      return decodeURIComponent(text);
    } catch {
      return text;
    }
  }
  function relativeFilePath(basePath, href) {
    const folder = href.startsWith("/") ? [] : basePath.split("/").slice(0, -1);
    const out = folder.filter(Boolean);
    for (const part of href.split("/")) {
      if (part === "" || part === ".")
        continue;
      if (part === "..")
        out.pop();
      else
        out.push(part);
    }
    return out.join("/");
  }
  function looksLikeSlug(target) {
    const bare = target.endsWith(".md") ? target.slice(0, -3) : target;
    if (!bare || bare.length > 80)
      return false;
    if (bare.includes("/") || bare.includes(".") || bare.includes(" "))
      return false;
    if (/^[0-9a-f]{7,40}$/i.test(bare))
      return false;
    return /^[A-Za-z0-9_-]+$/.test(bare);
  }
  function fileTargetUri(target) {
    let body = target;
    let rev = null;
    let line = null;
    const atIdx = body.indexOf("@");
    if (atIdx > 0) {
      const versionPart = body.slice(atIdx + 1);
      body = body.slice(0, atIdx);
      let token = versionPart;
      const colonIdx = versionPart.lastIndexOf(":");
      if (colonIdx >= 0 && /^\d+$/.test(versionPart.slice(colonIdx + 1))) {
        token = versionPart.slice(0, colonIdx);
        line = Number(versionPart.slice(colonIdx + 1));
      }
      const lower = token.toLowerCase();
      if (lower !== "disk" && lower !== "local" && token)
        rev = revisionSlot(gitRevision(token));
    }
    if (line === null) {
      const m = body.match(/^(.+?):(\d+)$/);
      if (m) {
        body = m[1];
        line = Number(m[2]);
      }
    }
    return line === null ? fileUri(body, rev) : fileLinesUri(body, line, line, rev);
  }
  function looksLikeFilePath(target) {
    const atIdx = target.indexOf("@");
    const path = atIdx > 0 ? target.slice(0, atIdx) : target;
    if (path.includes("/"))
      return true;
    const dot = path.lastIndexOf(".");
    if (dot <= 0)
      return false;
    const ext = path.slice(dot + 1).toLowerCase();
    return ext.length > 0 && ext !== "md" && /^[a-z0-9]+$/i.test(ext);
  }
  function preprocessWikilinks(body, mentionRef = () => null) {
    const segments = body.split(/(```[\s\S]*?```)/g);
    return segments.map((seg, idx) => {
      if (idx % 2 === 1)
        return seg;
      return rewriteWikilinksOutsideInlineCode(seg, mentionRef);
    }).join("");
  }
  function rewriteWikilinksOutsideInlineCode(text, mentionRef) {
    const parts = text.split(/(`[^`\n]*`)/g);
    return parts.map((part, idx) => {
      if (idx % 2 === 1)
        return part;
      return part.replace(/\[\[([^\[\]\n|]+)(?:\|([^\[\]\n]+))?\]\]/g, (_match, rawTarget, label) => {
        const target = rawTarget.trim();
        const display = (label ?? "").trim() || target;
        if (!target)
          return _match;
        const sha = parseGitRefTarget(target);
        if (sha) {
          const shortDisplay = label ? display : sha.slice(0, 7);
          return `[${shortDisplay}](${linkTarget(commitUri(sha))})`;
        }
        const dirTarget = target.startsWith("dir:") ? target.slice("dir:".length) : null;
        if (dirTarget !== null) {
          const dir = dirTarget.trim().replace(/\/+$/, "");
          if (dir) {
            const dirDisplay = label ? display : dir;
            return `[${dirDisplay}](${linkTarget(dirUri(dir))})`;
          }
        }
        const itemRef = taskId(target) !== null ? identityOf(target) ?? target : mentionRef(target);
        if (itemRef) {
          return `[${display}](${linkTarget(itemRef)})`;
        }
        const extensionRef = extensionWikilinkRef(target);
        if (extensionRef) {
          return `[${display}](${linkTarget(extensionRef)})`;
        }
        const colon = target.indexOf(":");
        if (colon > 0 && colon < target.length - 1) {
          const store = target.slice(0, colon);
          if (store === OXWIKI || enabledKnowledgeStoreIds().includes(store)) {
            return `[${display}](${linkTarget(knowledgeUri(store, target.slice(colon + 1)))})`;
          }
        }
        if (looksLikeFilePath(target)) {
          return `[${display}](${linkTarget(fileTargetUri(target))})`;
        }
        if (looksLikeSlug(target)) {
          const slug = target.endsWith(".md") ? target.slice(0, -3) : target;
          return `[${display}](${linkTarget(knowledgeUri("oxwiki", slug))})`;
        }
        return `[${display}](${INVALID_LINK}${encodeURIComponent(target)})`;
      });
    }).join("");
  }
  function refLinkNames(parsed) {
    return {
      knowledgeRef: parsed.kind === "knowledge" ? knowledgeUri(parsed.store, parsed.id) : null,
      itemRef: parsed.kind === "task" ? parsed.ref : null,
      extensionRef: parsed.kind === "ref" ? parsed.ref : null
    };
  }
  function linkRef(parsed) {
    switch (parsed.kind) {
      case "file":
        return withUriView(fileRef(parsed.path, parsed.version ?? WORKING, parsed.filesystem ?? null).id, { [RENDERER_VIEW_KEY]: parsed.renderer ?? null });
      case "directory":
        return directoryRef(parsed.path).id;
      case "commit":
        return gitCommitRef(parsed.sha).id;
      case "task":
      case "ref":
        return parsed.ref;
      case "knowledge":
        return knowledgePageRef(parsed.store, parsed.id).id;
      default:
        return null;
    }
  }

  // src/components/Markdown/markdownTree.ts
  var BLOCK_ELEMENTS = ["p", "ul", "ol", "li", "blockquote", "pre", "h1", "h2", "h3", "h4", "h5", "h6", "hr"];
  var parser = unified().use(remarkParse).use(remarkGfm).use(remarkRehype);
  async function markdownTree(text, inline, deps) {
    const hast = await parser.run(parser.parse(preprocessWikilinks(text, deps.mentionRef)));
    const allowed = new Set(inline ? INLINE_ELEMENTS : [...INLINE_ELEMENTS, ...BLOCK_ELEMENTS]);
    const nodes = merge(await convertAll(hast.children, allowed, deps), inline);
    return inline ? trimLine2(nodes) : nodes.filter((n) => typeof n !== "string" || n.trim() !== "");
  }
  async function convertAll(children, allowed, deps) {
    return (await Promise.all(children.map((c) => convert2(c, allowed, deps)))).flat();
  }
  async function convert2(node, allowed, deps) {
    if (node.type === "text")
      return [node.value];
    if (node.type !== "element")
      return [];
    const children = merge(await convertAll(node.children, allowed, deps), false);
    if (node.tagName === "a")
      return [await link3(node, children, deps)];
    if (!allowed.has(node.tagName))
      return children;
    const kept = BLOCK_ELEMENTS.includes(node.tagName) && node.tagName !== "pre" ? children.filter((c) => typeof c !== "string" || c.trim() !== "") : children;
    return [{ tag: node.tagName, children: kept }];
  }
  async function link3(node, children, deps) {
    const href = urlTransform(typeof node.properties?.href === "string" ? node.properties.href : "");
    if (!href)
      return { tag: "span", children };
    const parsed = parseMarkdownLink(href);
    if (parsed.kind === "broken")
      return { tag: "a", broken: parsed.reason, children };
    const ref = linkRef(parsed);
    if (ref) {
      const text = children.length === 1 && typeof children[0] === "string" ? children[0] : null;
      const { title, brokenReason } = await deps.refTitle(refLinkNames(parsed), text ?? "");
      if (brokenReason)
        return { tag: "a", broken: brokenReason, children };
      return { tag: "a", ref, children: title ? [title] : children };
    }
    if (/^https?:\/\//i.test(href))
      return { tag: "a", href, children };
    return { tag: "span", children };
  }
  function merge(nodes, line) {
    const out = [];
    for (const n of nodes.flatMap((n) => typeof n !== "string" && n.tag === "span" ? n.children : [n])) {
      const s = typeof n === "string" && line ? n.replace(/\s+/g, " ") : n;
      const last = out[out.length - 1];
      if (typeof s === "string" && typeof last === "string")
        out[out.length - 1] = line ? (last + s).replace(/\s+/g, " ") : last + s;
      else
        out.push(s);
    }
    return out;
  }
  function trimLine2(nodes) {
    const out = [...nodes];
    if (typeof out[0] === "string")
      out[0] = out[0].trimStart();
    const end = out.length - 1;
    if (typeof out[end] === "string")
      out[end] = out[end].trimEnd();
    return out.filter((n) => n !== "");
  }
  var NO_LOOKUPS = {
    mentionRef: () => null,
    refTitle: async () => ({ title: null, brokenReason: null })
  };

  // src/viz/harnessHost.ts
  function fedBy(run, node) {
    const def = nodeById(run.page, node);
    if (def?.kind === "viz")
      return pageData(run, def.config.custom?.props ?? null, nodeRunOf(run, def));
    return pageData(run, def?.kind === "component" ? def.props : null);
  }
  function drawsFrames(wait = 2000) {
    return new Promise((drew) => {
      const timer = setTimeout(() => drew(false), wait);
      requestAnimationFrame(() => {
        clearTimeout(timer);
        drew(true);
      });
    });
  }
  async function runHarness(server, frame, agent, frames = drawsFrames) {
    const drawing = await frames();
    server.record("drawing", drawing);
    if (!drawing) {
      while ((await server.next(null)).op !== "done")
        ;
      return;
    }
    const start = await server.start();
    const fed = fedBy(start.run, start.node);
    let showing = Promise.resolve();
    const host = await new Promise((mounted) => {
      frame.addEventListener("load", () => {
        const channel = new MessageChannel;
        const record = (kind) => (value) => server.record(kind, value);
        const bridge = createBridgeHost(channel.port1, {
          invoke: async (command, input, confirmed) => ({ result: await server.rpc("invoke", { command, input, confirmed }) }),
          navigate: (ref) => {
            server.record("navigate", ref);
            return true;
          },
          confirm: async (command) => await server.rpc("confirm", { command }) === true,
          markdown: (text, inline) => markdownTree(text, inline, NO_LOOKUPS),
          onReady: () => server.record("ready", true),
          querySql: async (sql, params) => await server.rpc("querySql", { sql, params }),
          openForm: async (command, input) => await server.rpc("openForm", { command, input }),
          askForm: async (form) => await server.rpc("askForm", { form }),
          showView: (spec) => {
            const mine = showing.then(async () => {
              if (spec === null) {
                server.record("showView", null);
                return null;
              }
              try {
                const drawn = await server.rpc("showView", { spec });
                server.record("showView", spec);
                return drawn;
              } catch (e) {
                server.record("showView", null);
                throw e;
              }
            });
            showing = mine.then(() => {
              return;
            }, () => {
              return;
            });
            return mine;
          },
          setTitle: record("title"),
          setActions: record("actions"),
          setViewParams: record("viewParams"),
          setHistoryState: record("historyState")
        }, fed, start.view);
        frame.contentWindow?.postMessage(initMessage(fed, {}, start.page, start.view), "*", [channel.port2]);
        mounted(bridge);
      }, { once: true });
      frame.src = `/components/v/${start.version}/`;
    });
    const pending = new Map;
    agent.answers((id, result) => {
      pending.get(id)?.(result);
      pending.delete(id);
    });
    let ids = 0;
    let result = null;
    for (;; ) {
      const op = await server.next(result);
      switch (op.op) {
        case "dom": {
          const id = ++ids;
          result = await new Promise((answered) => {
            pending.set(id, answered);
            agent.send(id, op.action);
          });
          break;
        }
        case "press":
          host.action(op.id);
          result = null;
          break;
        case "viewParams":
          host.viewParams(op.params);
          result = null;
          break;
        case "update":
          host.update(fedBy(op.run, start.node));
          result = null;
          break;
        case "done":
          host.close();
          return;
      }
    }
  }
  function httpServer() {
    let sent = Promise.resolve();
    const post = async (path, body) => {
      const response = await fetch(path, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify(body)
      });
      return response.json();
    };
    return {
      start: async () => (await fetch("/harness/start")).json(),
      rpc: async (method, body) => {
        const out = await post("/harness/rpc", { method, ...body });
        if ("error" in out)
          throw new IpcCallError(out.error.message, out.error.code, out.error.field ?? null);
        return out.result;
      },
      record: (kind, value) => {
        sent = sent.then(() => fetch("/harness/record", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ kind, value })
        }).then(() => {
          return;
        }, () => {
          return;
        }));
      },
      next: async (result) => {
        await sent;
        return await post("/harness/next", { result });
      }
    };
  }
  function main() {
    const frame = document.getElementById("frame");
    runHarness(httpServer(), frame, {
      send: (id, action) => frame.contentWindow?.postMessage({ oxplowHarness: id, action }, "*"),
      answers: (cb) => window.addEventListener("message", (e) => {
        if (e.source !== frame.contentWindow)
          return;
        const data = e.data;
        if (typeof data?.oxplowHarness === "number")
          cb(data.oxplowHarness, data.result);
      })
    }).then(() => {
      location.href = "/harness/done";
    });
  }

  // src/viz/harnessEntry.ts
  main();
})();
