// The component harness's agent (`oxplow extension test`): put first in a
// bundle's page as the harness serves it, it does what the test's steps
// ask of the frame — reads what is drawn, clicks, types, drags — when the
// harness's host page asks (`{ oxplowHarness: id, action }`), and answers
// it (`{ oxplowHarness: id, result }`). It also keeps what went wrong in
// the page (an uncaught error, a rejected promise, `console.error`) for a
// finding to show. Input is synthetic (dispatched DOM events), not a
// person's: focus and keyboard details can differ.
(function () {
  "use strict";
  var errors = [];
  window.addEventListener("error", function (e) {
    errors.push(String(e.message || e.error));
  });
  window.addEventListener("unhandledrejection", function (e) {
    errors.push("unhandled rejection: " + String(e.reason && e.reason.message ? e.reason.message : e.reason));
  });
  var logged = console.error;
  console.error = function () {
    errors.push(Array.prototype.map.call(arguments, String).join(" "));
    return logged.apply(console, arguments);
  };

  function all(selector) {
    return Array.prototype.slice.call(document.querySelectorAll(selector));
  }
  function one(selector) {
    var el = document.querySelector(selector);
    if (!el) throw new Error("nothing matches `" + selector + "`");
    return el;
  }
  function mouse(el, type, init) {
    el.dispatchEvent(new MouseEvent(type, Object.assign({ bubbles: true, cancelable: true, view: window }, init || {})));
  }
  function setValue(el, value) {
    // The element's own setter, so a framework watching `value` sees it.
    var proto = Object.getPrototypeOf(el);
    var setter = Object.getOwnPropertyDescriptor(proto, "value");
    if (setter && setter.set) setter.set.call(el, value);
    else el.value = value;
    el.dispatchEvent(new Event("input", { bubbles: true }));
    el.dispatchEvent(new Event("change", { bubbles: true }));
  }
  function key(el, name) {
    var parts = name.split("+");
    var k = parts.pop();
    var mod = function (m) {
      return parts.indexOf(m) >= 0;
    };
    var init = {
      key: k,
      bubbles: true,
      cancelable: true,
      ctrlKey: mod("Ctrl") || (mod("Mod") && !/Mac/.test(navigator.platform)),
      metaKey: mod("Meta") || (mod("Mod") && /Mac/.test(navigator.platform)),
      shiftKey: mod("Shift"),
      altKey: mod("Alt"),
    };
    el.dispatchEvent(new KeyboardEvent("keydown", init));
    el.dispatchEvent(new KeyboardEvent("keyup", init));
  }
  function drag(from, to) {
    var data = new DataTransfer();
    from.dispatchEvent(new DragEvent("dragstart", { bubbles: true, cancelable: true, dataTransfer: data }));
    to.dispatchEvent(new DragEvent("dragenter", { bubbles: true, cancelable: true, dataTransfer: data }));
    to.dispatchEvent(new DragEvent("dragover", { bubbles: true, cancelable: true, dataTransfer: data }));
    to.dispatchEvent(new DragEvent("drop", { bubbles: true, cancelable: true, dataTransfer: data }));
    from.dispatchEvent(new DragEvent("dragend", { bubbles: true, cancelable: true, dataTransfer: data }));
  }

  function act(a) {
    if (a.query !== undefined) {
      var found = all(a.query);
      return {
        count: found.length,
        texts: found.map(function (el) {
          return (el.textContent || "").trim();
        }),
      };
    }
    if (a.click !== undefined) {
      var el = one(a.click);
      mouse(el, "mousedown");
      mouse(el, "mouseup");
      // An SVG element has no `click()`: it hears the event.
      if (typeof el.click === "function") el.click();
      else mouse(el, "click");
      return null;
    }
    if (a.fill !== undefined) {
      Object.keys(a.fill).forEach(function (selector) {
        var target = one(selector);
        if (target.focus) target.focus();
        setValue(target, String(a.fill[selector]));
      });
      return null;
    }
    if (a.key !== undefined) {
      key(one(a.key.on), a.key.key);
      return null;
    }
    if (a.drag !== undefined) {
      drag(one(a.drag.from), one(a.drag.to));
      return null;
    }
    if (a.page !== undefined) {
      return { html: document.documentElement.outerHTML.slice(0, 4096), errors: errors.slice() };
    }
    throw new Error("the harness asked for nothing it knows: " + JSON.stringify(a));
  }

  window.addEventListener("message", function (e) {
    if (e.source !== window.parent) return;
    var d = e.data;
    if (!d || typeof d.oxplowHarness !== "number") return;
    var result;
    try {
      result = { ok: true, value: act(d.action) };
    } catch (err) {
      result = { ok: false, error: String(err && err.message ? err.message : err) };
    }
    window.parent.postMessage({ oxplowHarness: d.oxplowHarness, result: result }, "*");
  });
})();
