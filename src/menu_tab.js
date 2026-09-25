// Injected into Garry's Mod's HTML main menu by gmm_workshop.lua.
// Adds a "GMod Manager" entry to the Addons page listing the active preset's mods.
// Uses GMod's own workshop card elements so it matches the rest of the menu.
window.GMMInit = function (data, states) {
  var G = window.GMM || (window.GMM = { active: false });
  G.data = data;
  G.states = states || [];

  var STATE_TEXT = {
    local: "Installed from the GMod Manager library.",
    steam: "Mounted through Steam.",
    downloading: "Waiting for Steam to download or mount this mod. Stay in the main menu before starting a map.",
    missing: "Not downloaded yet. Press Play in GMod Manager."
  };

  function page() {
    return document.querySelector('div.page[ng-controller="ControllerAddons"]');
  }

  // classList.toggle's second argument isn't available in GMod's older Awesomium engine.
  function toggleClass(element, name, on) {
    if (on) element.classList.add(name);
    else element.classList.remove(name);
  }

  function setActive(root, on) {
    G.active = on;
    toggleClass(root, "gmm-on", on);
    var link = root.querySelector("a.gmm-link");
    if (link) toggleClass(link, "gmm-active", on);
    if (on) render();
  }

  function render() {
    var root = page();
    if (!root || !root.getAttribute("data-gmm")) return;
    var list = root.querySelector("workshopcontainer.gmm-list");
    var title = root.querySelector("h1.gmm-header span");
    var subtitle = root.querySelector("h1.gmm-header small");
    var d = G.data;
    title.textContent = "GMod Manager";
    var waiting = G.states.filter(function (state) { return state !== "local" && state !== "steam"; }).length;
    subtitle.textContent = d.preset + " \u00b7 " + d.mods.length + " mods" +
      (d.size ? " \u00b7 " + d.size : "") +
      (waiting ? " \u00b7 " + waiting + " not ready (wait before starting a map)" : "") +
      " \u00b7 Modify in GMM app";
    while (list.firstChild) list.removeChild(list.firstChild);
    if (!d.mods.length) {
      var empty = document.createElement("workshopmessage");
      empty.textContent = "This preset has no mods.";
      list.appendChild(empty);
      return;
    }
    var width = Math.max(180, list.clientWidth - 16);
    var perRow = Math.min(6, Math.max(1, Math.floor(width / 180)));
    var size = Math.floor(width / perRow) - 26;
    var half = -Math.floor((size + 1) / 2) + "px";
    d.mods.forEach(function (mod, index) {
      var state = G.states[index] || "missing";
      var card = document.createElement("workshopicon");
      card.className = state === "local" || state === "steam" ? "installed" : "disabled";
      card.style.width = size + "px";
      card.style.height = size + "px";

      var preview = document.createElement("preview");
      preview.style.width = preview.style.height = size + 1 + "px";
      preview.style.marginLeft = preview.style.marginTop = half;
      if (mod.preview) {
        var img = document.createElement("img");
        img.src = mod.preview;
        img.style.width = img.style.height = size + 1 + "px";
        img.setAttribute("loading", "lazy");
        preview.appendChild(img);
      }
      preview.appendChild(document.createElement("disabled"));
      card.appendChild(preview);

      var name = document.createElement("name");
      var label = document.createElement("label");
      label.textContent = mod.title;
      label.title = "Open on the Workshop";
      label.addEventListener("click", function () {
        lua.Run("steamworks.ViewFile( %s )", String(mod.id));
      });
      name.appendChild(label);
      card.appendChild(name);

      if (mod.size) {
        var bytes = document.createElement("size");
        bytes.textContent = mod.size;
        card.appendChild(bytes);
      }

      var description = document.createElement("description");
      description.textContent = STATE_TEXT[state] || "";
      var hint = document.createElement("b");
      hint.textContent = "Modify in GMM app";
      description.appendChild(hint);
      card.appendChild(description);

      list.appendChild(card);
    });
  }

  function attach() {
    var root = page();
    if (!root || root.getAttribute("data-gmm")) return;
    var nav = root.querySelector("div.options > ul");
    var content = root.querySelector("div.ugc_content");
    if (!nav || !content || nav.children.length < 2) return;
    root.setAttribute("data-gmm", "1");
    G.active = false;

    var item = document.createElement("li");
    var link = document.createElement("a");
    link.className = "gmm-link";
    link.textContent = "GMod Manager";
    link.addEventListener("click", function () { setActive(root, true); });
    item.appendChild(link);
    nav.insertBefore(item, nav.children[1].nextSibling);
    nav.addEventListener("click", function (event) {
      if (!item.contains(event.target)) setActive(root, false);
    }, true);

    var header = document.createElement("h1");
    header.className = "menuheader gmm-header";
    header.appendChild(document.createElement("span"));
    header.appendChild(document.createTextNode(" "));
    header.appendChild(document.createElement("small"));
    content.insertBefore(header, content.firstChild);

    var list = document.createElement("workshopcontainer");
    list.className = "gmm-list";
    content.appendChild(list);
  }

  if (!G.observer) {
    var style = document.createElement("style");
    style.textContent =
      "div.page.gmm-on div.options a.active { color: #fff; }" +
      "div.options a.gmm-link { cursor: pointer; }" +
      "div.options a.gmm-link.gmm-active { color: #ff5; }" +
      ".gmm-header, .gmm-list { display: none; }" +
      ".gmm-on .ugc_content > h1.menuheader, .gmm-on .ugc_content > workshopcontainer," +
      " .gmm-on .ugc_content > center, .gmm-on .ugc_content > .ugc_settings_button," +
      " .gmm-on .ugc_content > .ugc_settings { display: none !important; }" +
      ".gmm-on .ugc_content > h1.gmm-header { display: block !important; }" +
      ".gmm-on .ugc_content > workshopcontainer.gmm-list { display: block !important; overflow-y: auto; }" +
      ".gmm-list workshopicon { cursor: default; }" +
      ".gmm-list description b { display: block; margin-top: 8px; }";
    document.head.appendChild(style);
    var Observer = window.MutationObserver || window.WebKitMutationObserver;
    if (Observer) {
      G.observer = new Observer(attach);
      G.observer.observe(document.body, { childList: true, subtree: true });
    } else {
      G.observer = setInterval(attach, 500);
    }
    window.addEventListener("resize", function () { if (G.active) render(); });
  }
  attach();
  if (G.active) render();
};
