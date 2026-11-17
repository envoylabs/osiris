// A stand-in for the parts of a browser the map page's script uses, so
// the script runs in a test with no browser. It is not a DOM: elements
// hold their children, text, class, attributes and listeners, and an
// event bubbles to the element's parents. d3 is replaced by a stub that
// accepts any chain of calls and draws nothing, so the graph's drawing
// is not exercised here; the list, the view switch and the pane are
// plain DOM and are.
//
// `__build(elements, data)` makes the page's elements from its markup:
// each is `{ tag, attrs }`, and `data` is the text of the element that
// holds the document. After the page's script has run, the `__` helpers
// below act on the page and read it back as JSON.

function El(tag) {
  this.tagName = tag.toUpperCase();
  this.childNodes = [];
  this.parentNode = null;
  this.className = "";
  this.attrs = {};
  this.listeners = {};
  this.style = {};
  this.hidden = false;
  this.tabIndex = -1;
  this.scrollTop = 0;
  this.scrollLeft = 0;
  this.offsetWidth = 0;
  this.offsetHeight = 0;
  this._text = "";
  var self = this;
  var names = function () {
    return self.className.split(" ").filter(function (c) { return c !== ""; });
  };
  this.classList = {
    contains: function (c) { return names().indexOf(c) >= 0; },
    toggle: function (c, on) {
      var has = names().indexOf(c) >= 0;
      var want = on === undefined ? !has : Boolean(on);
      if (want && !has) { self.className = names().concat([c]).join(" "); }
      if (!want && has) {
        self.className = names().filter(function (d) { return d !== c; }).join(" ");
      }
      return want;
    }
  };
}
Object.defineProperty(El.prototype, "textContent", {
  get: function () {
    return this._text + this.childNodes.map(function (c) { return c.textContent; }).join("");
  },
  set: function (text) {
    this.childNodes.forEach(function (c) { c.parentNode = null; });
    this.childNodes = [];
    this._text = String(text);
  }
});
El.prototype.appendChild = function (child) {
  child.parentNode = this;
  this.childNodes.push(child);
  return child;
};
El.prototype.setAttribute = function (name, value) {
  if (name === "class") { this.className = String(value); return; }
  this.attrs[name] = String(value);
};
El.prototype.getAttribute = function (name) {
  if (name === "class") { return this.className; }
  return name in this.attrs ? this.attrs[name] : null;
};
El.prototype.addEventListener = function (type, fn) {
  (this.listeners[type] = this.listeners[type] || []).push(fn);
};
El.prototype.getBoundingClientRect = function () {
  return { top: 0, bottom: 40, left: 0, right: 0, width: 0, height: 0 };
};
// Every element below this one, in document order.
El.prototype.all = function () {
  var out = [];
  this.childNodes.forEach(function (c) { out.push(c); out = out.concat(c.all()); });
  return out;
};

var __ids = {};
var __tags = {};
var __window = { listeners: {} };

var document = {
  body: null,
  getElementById: function (id) { return __ids.hasOwnProperty(id) ? __ids[id] : null; },
  createElement: function (tag) { return new El(tag); },
  querySelector: function (sel) {
    if (sel.charAt(0) === "#") { return document.getElementById(sel.slice(1)); }
    return __tags.hasOwnProperty(sel) ? __tags[sel] : null;
  }
};
var window = {
  innerWidth: 1200,
  innerHeight: 800,
  addEventListener: function (type, fn) {
    (__window.listeners[type] = __window.listeners[type] || []).push(fn);
  }
};

var d3 = (function () {
  var chain = new Proxy(function () {}, {
    get: function (target, key) {
      if (key === Symbol.toPrimitive) { return function () { return 0; }; }
      return chain;
    },
    apply: function () { return chain; }
  });
  var max = function (values, of) {
    var most;
    values.forEach(function (v) {
      var x = of(v);
      if (most === undefined || x > most) { most = x; }
    });
    return most;
  };
  return new Proxy({}, {
    get: function (target, key) { return key === "max" ? max : chain; }
  });
}());

function __build(elements, data) {
  elements.forEach(function (e) {
    var el = new El(e.tag);
    Object.keys(e.attrs).forEach(function (k) { el.setAttribute(k, e.attrs[k]); });
    if (e.attrs.hidden !== undefined) { el.hidden = true; }
    if (e.attrs.id !== undefined) { __ids[e.attrs.id] = el; }
    if (!__tags.hasOwnProperty(e.tag)) { __tags[e.tag] = el; }
    if (e.tag === "body") { document.body = el; }
    if (e.attrs.id === "map-data") { el.textContent = data; }
  });
}

// Fires `type` at `el`, then at each of its parents until a listener
// stops it, as a click or a key press bubbles in a browser.
function __fire(el, type, props) {
  var event = {
    type: type,
    target: el,
    defaultPrevented: false,
    stopped: false,
    stopPropagation: function () { this.stopped = true; },
    preventDefault: function () { this.defaultPrevented = true; }
  };
  Object.keys(props || {}).forEach(function (k) { event[k] = props[k]; });
  for (var at = el; at !== null && !event.stopped; at = at.parentNode) {
    (at.listeners[type] || []).slice().forEach(function (fn) { fn.call(at, event); });
  }
}

function __rows() {
  return document.getElementById("list-rows").childNodes.filter(function (tr) {
    return tr.classList.contains("file");
  });
}

// The lines an expanded row shows: each marked line and each step.
function __marks() {
  var out = [];
  document.getElementById("list-rows").childNodes.forEach(function (tr) {
    if (!tr.classList.contains("marks")) { return; }
    tr.childNodes.forEach(function (td) { out = out.concat(td.childNodes); });
  });
  return out;
}
// Those of them that open a function in the calls view.
function __opening() {
  return __marks().filter(function (div) { return div.classList.contains("go"); });
}

function __click(id) { __fire(document.getElementById(id), "click"); }
function __clickMark(i) { __fire(__opening()[i], "click"); }
function __clickRow(i) { __fire(__rows()[i], "click"); }
function __key(key) {
  (__window.listeners.keydown || []).forEach(function (fn) { fn({ key: key }); });
}

// What a reader of the page sees, as JSON.
function __state() {
  var pressed = {};
  ["list", "graph", "calls"].forEach(function (v) {
    pressed[v] = document.getElementById("view-" + v).getAttribute("aria-pressed");
  });
  var body = document.getElementById("pane-body");
  return JSON.stringify({
    body: document.body.className,
    pressed: pressed,
    rows: __rows().map(function (tr) {
      return {
        path: tr.all().filter(function (e) { return e.className === "path"; })[0].textContent,
        picked: tr.classList.contains("picked")
      };
    }),
    marks: __marks().map(function (div) {
      return { text: div.textContent, opens: div.classList.contains("go") };
    }),
    list_top: document.getElementById("list").style.top || "",
    pane_path: document.getElementById("pane-path").textContent,
    pane_rows: body.all().filter(function (e) {
      return e.className.indexOf("row ") === 0;
    }).map(function (e) { return e.className.slice(4); }),
    pane_text: body.textContent,
    calls_note: document.getElementById("calls-note").textContent
  });
}
